//! The platform modules working together over memory adapters.

use std::sync::Arc;
use std::time::Duration;

use igloo_core::job::{JobFailure, JobPhase, JobSpec, JobTimeout};
use igloo_core::process::{Argv, EnvVars};
use igloo_core::repo::{RepoLocation, SecretName};
use igloo_core::sandbox::{
    DesiredState, FailureReason, Isolation, SandboxId, SandboxPhase, SandboxSpec,
};
use igloo_core::seal::{SealFailure, SealId, SealPhase};
use igloo_core::snapshot::{MediaType, SnapshotId, SnapshotLayer};
use igloo_core::worker::{Arch, Capabilities, Os, ProtocolVersion, RuntimeKind, WorkerId};
use igloo_core::{Digest, ErrorCode, Generation, Id, Labels, Resource};
use uuid::Uuid;

use super::*;
use crate::app::{CommandBus, ControllerSettings, TaskSupervisor};
use crate::ports::conformance::ImageRegistryConformance;
use crate::ports::{EventLog, ImageRegistry, SecretValue};
use crate::testing::{FIXTURE_IMAGE, MemoryPlatform, MemoryStores, context, wait_until};

struct Running {
    stores: MemoryStores,
    forge: Arc<dyn crate::ports::Forge>,
    registry: Arc<dyn ImageRegistry>,
    bus: CommandBus,
    supervisor: TaskSupervisor,
}

fn start() -> Running {
    let MemoryPlatform {
        mut builder,
        stores,
    } = MemoryPlatform::new();
    builder
        .install(SandboxModule {
            settings: ControllerSettings::default(),
        })
        .expect("sandbox module");
    builder
        .install(WorkerModule {
            settings: ControllerSettings::default(),
        })
        .expect("worker module");
    builder
        .install(JobModule {
            settings: ControllerSettings::default(),
        })
        .expect("job module");
    builder.install(SnapshotModule).expect("snapshot module");
    builder.install(SealModule).expect("seal module");
    builder.install(RepoModule).expect("repo module");
    let registry = Arc::clone(&builder.ports().registry);
    let forge = Arc::clone(&builder.ports().forge);
    let supervisor = TaskSupervisor::new();
    let bus = builder.build().start(&supervisor);
    Running {
        stores,
        forge,
        registry,
        bus,
        supervisor,
    }
}

impl Running {
    async fn register_worker(&self) -> WorkerId {
        self.register_worker_with(RuntimeKind::Process).await
    }

    async fn register_worker_with(&self, runtime: RuntimeKind) -> WorkerId {
        let capabilities = Capabilities::new(
            Os::Linux,
            Arch::X86_64,
            [runtime].into(),
            ProtocolVersion::V1,
        )
        .expect("one runtime");
        self.bus
            .dispatch(
                RegisterWorker {
                    capabilities,
                    labels: Labels::default(),
                },
                context(),
            )
            .await
            .expect("register worker")
    }

    async fn snapshot(&self) -> SnapshotId {
        let layer = b"working tree".to_vec();
        let digest = Digest::from_blake3(*blake3::hash(&layer).as_bytes());
        self.bus
            .dispatch(
                StoreBlob {
                    digest,
                    data: Box::pin(std::io::Cursor::new(layer)),
                },
                context(),
            )
            .await
            .expect("store layer");
        let register = RegisterSnapshot {
            base: None,
            layers: vec![SnapshotLayer::new(digest, MediaType::Tar)],
            env: igloo_core::process::EnvVars::default(),
        };
        self.bus
            .dispatch(register, context())
            .await
            .expect("register snapshot")
    }

    async fn create_sandbox(&self) -> SandboxId {
        let spec = SandboxSpec::builder()
            .snapshot(self.snapshot().await)
            .build();
        self.bus
            .dispatch(CreateSandbox { spec }, context())
            .await
            .expect("create sandbox")
    }

    async fn sandbox_phase(&self, id: SandboxId) -> (SandboxPhase, Option<WorkerId>) {
        let sandbox = self
            .stores
            .sandboxes
            .load(id)
            .await
            .expect("load")
            .expect("exists");
        let status = *sandbox.entity().status();
        (status.phase(), status.worker())
    }

    async fn wait_until_placed(&self, id: SandboxId) -> WorkerId {
        wait_until(async || self.sandbox_phase(id).await.1.is_some()).await;
        self.sandbox_phase(id).await.1.expect("placed")
    }

    async fn record(&self, sandbox: SandboxId, phase: SandboxPhase, generation: u64) {
        let command = RecordSandboxStatus {
            sandbox,
            phase,
            observed_generation: Generation::try_from(generation).expect("non-zero"),
        };
        self.bus.dispatch(command, context()).await.expect("record");
    }

    async fn seal(&self, sandbox: SandboxId) -> SealId {
        self.bus
            .dispatch(CreateSeal { sandbox }, context())
            .await
            .expect("seal")
    }

    async fn complete_seal(&self, seal: SealId, layer: SnapshotLayer, full: bool) -> SnapshotId {
        self.bus
            .dispatch(CompleteSeal { seal, layer, full }, context())
            .await
            .expect("complete")
    }

    async fn seal_phase(&self, seal: SealId) -> SealPhase {
        self.stores
            .seals
            .load(seal)
            .await
            .expect("load")
            .expect("seal")
            .entity()
            .phase()
    }

    /// The paths in the stored layer `digest`.
    async fn layer_entries(&self, digest: Digest) -> Vec<String> {
        let mut reader = self
            .stores
            .blobs
            .get(digest)
            .await
            .expect("get")
            .expect("layer");
        let mut bytes = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut bytes)
            .await
            .expect("read");
        tar::Archive::new(bytes.as_slice())
            .entries()
            .expect("entries")
            .map(|entry| {
                entry
                    .expect("entry")
                    .path()
                    .expect("path")
                    .display()
                    .to_string()
            })
            .collect()
    }

    async fn stop(self) {
        self.supervisor
            .shutdown(Duration::from_secs(1))
            .await
            .expect("shutdown");
    }
}

#[tokio::test(start_paused = true)]
async fn a_created_sandbox_is_placed_on_a_capable_worker() {
    let running = start();
    let worker = running.register_worker().await;
    let sandbox = running.create_sandbox().await;
    assert_eq!(running.wait_until_placed(sandbox).await, worker);
    assert_eq!(
        running.sandbox_phase(sandbox).await.0,
        SandboxPhase::Scheduled
    );
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn an_isolated_sandbox_waits_for_a_worker_with_the_oci_runtime() {
    let running = start();
    running.register_worker().await;
    let spec = SandboxSpec::builder()
        .snapshot(running.snapshot().await)
        .isolation(Isolation::Container)
        .build();
    let sandbox = running
        .bus
        .dispatch(CreateSandbox { spec }, context())
        .await
        .expect("create sandbox");
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(
        running.sandbox_phase(sandbox).await,
        (SandboxPhase::Pending, None),
        "a process worker cannot host it"
    );
    let oci = running.register_worker_with(RuntimeKind::Oci).await;
    assert_eq!(running.wait_until_placed(sandbox).await, oci);
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn without_a_capable_worker_a_sandbox_waits_until_one_registers() {
    let running = start();
    let sandbox = running.create_sandbox().await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(
        running.sandbox_phase(sandbox).await,
        (SandboxPhase::Pending, None)
    );
    let worker = running.register_worker().await;
    assert_eq!(running.wait_until_placed(sandbox).await, worker);
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn stopping_a_sandbox_updates_its_workers_assignment() {
    let running = start();
    let worker = running.register_worker().await;
    let sandbox = running.create_sandbox().await;
    running.wait_until_placed(sandbox).await;
    running.record(sandbox, SandboxPhase::Running, 1).await;
    let queries = SandboxQueries::new(running.stores.sandboxes.clone());
    let assigned = queries.on_worker(worker).await.expect("assignment");
    assert_eq!(assigned.len(), 1);
    assert_eq!(assigned[0].spec().desired(), DesiredState::Running);
    running
        .bus
        .dispatch(StopSandbox { sandbox }, context())
        .await
        .expect("stop");
    let assigned = queries.on_worker(worker).await.expect("assignment");
    assert_eq!(assigned[0].spec().desired(), DesiredState::Stopped);
    assert_eq!(assigned[0].generation(), Generation::INITIAL.next());
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_lost_worker_fails_its_sandboxes() {
    let running = start();
    let worker = running.register_worker().await;
    let sandbox = running.create_sandbox().await;
    running.wait_until_placed(sandbox).await;
    running.record(sandbox, SandboxPhase::Running, 1).await;
    running
        .bus
        .dispatch(DisconnectWorker { worker }, context())
        .await
        .expect("disconnect");
    let failed = SandboxPhase::Failed {
        reason: FailureReason::WorkerLost,
    };
    wait_until(async || running.sandbox_phase(sandbox).await.0 == failed).await;
    running.stop().await;
}

fn job_spec(sandbox: SandboxId) -> JobSpec {
    JobSpec::Execute {
        sandbox,
        argv: Argv::try_from(vec!["cargo".to_owned(), "test".to_owned()]).expect("valid"),
        env: EnvVars::default(),
        secrets: std::collections::BTreeSet::new(),
        timeout: JobTimeout::default(),
    }
}

#[tokio::test(start_paused = true)]
async fn jobs_queue_for_their_sandboxes_worker_and_end_with_the_sandbox() {
    let running = start();
    let worker = running.register_worker().await;
    let sandbox = running.create_sandbox().await;
    running.wait_until_placed(sandbox).await;
    let job = running
        .bus
        .dispatch(
            SubmitJob {
                spec: job_spec(sandbox),
            },
            context(),
        )
        .await
        .expect("submit");
    let queries = JobQueries::new(
        running.stores.jobs.clone(),
        running.stores.sandboxes.clone(),
    );
    let queued: Vec<_> = queries
        .queued_for(worker)
        .await
        .expect("queued")
        .iter()
        .map(igloo_core::Entity::id)
        .collect();
    assert_eq!(queued, [job]);
    let lease = running
        .bus
        .dispatch(
            LeaseJob {
                job,
                worker,
                duration: jiff::SignedDuration::from_secs(30),
            },
            context(),
        )
        .await
        .expect("lease");
    assert_eq!(lease.worker(), worker);
    assert_eq!(queries.queued_for(worker).await.expect("queued").len(), 0);
    running.record(sandbox, SandboxPhase::Stopped, 1).await;
    let jobs = running.stores.jobs.clone();
    wait_until(async || {
        let job = jobs.load(job).await.expect("load").expect("exists");
        job.entity().phase() == JobPhase::Cancelled
    })
    .await;
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_job_whose_lease_expires_fails_with_lease_lost_unless_renewed() {
    let running = start();
    let worker = running.register_worker().await;
    let sandbox = running.create_sandbox().await;
    running.wait_until_placed(sandbox).await;
    let job = running
        .bus
        .dispatch(
            SubmitJob {
                spec: job_spec(sandbox),
            },
            context(),
        )
        .await
        .expect("submit");
    let lease = running
        .bus
        .dispatch(
            LeaseJob {
                job,
                worker,
                duration: jiff::SignedDuration::from_secs(10),
            },
            context(),
        )
        .await
        .expect("lease");
    tokio::time::sleep(Duration::from_secs(8)).await;
    running
        .bus
        .dispatch(
            RenewJobLease {
                job,
                token: lease.token(),
                duration: jiff::SignedDuration::from_secs(10),
            },
            context(),
        )
        .await
        .expect("renew");
    tokio::time::sleep(Duration::from_secs(8)).await;
    let jobs = running.stores.jobs.clone();
    let phase = jobs
        .load(job)
        .await
        .expect("load")
        .expect("exists")
        .entity()
        .phase();
    assert_eq!(phase, JobPhase::Leased, "a renewed lease holds");

    wait_until(async || {
        let job = jobs.load(job).await.expect("load").expect("exists");
        job.entity().phase()
            == JobPhase::Failed {
                reason: JobFailure::LeaseLost,
            }
    })
    .await;
    let late = running
        .bus
        .dispatch(
            FinishJob {
                job,
                token: lease.token(),
                exit_code: 0,
            },
            context(),
        )
        .await
        .expect_err("a late result is refused");
    assert_eq!(late.code(), "job.invalid_transition");
    running.stop().await;
}

#[tokio::test]
async fn jobs_need_a_live_sandbox() {
    let running = start();
    let unknown = Id::from_uuid(Uuid::from_u128(999));
    let error = running
        .bus
        .dispatch(
            SubmitJob {
                spec: job_spec(unknown),
            },
            context(),
        )
        .await
        .expect_err("unknown sandbox");
    assert_eq!(error.code(), "sandbox.not_found");
    let sandbox = running.create_sandbox().await;
    running
        .bus
        .dispatch(StopSandbox { sandbox }, context())
        .await
        .expect("stop");
    let error = running
        .bus
        .dispatch(
            SubmitJob {
                spec: job_spec(sandbox),
            },
            context(),
        )
        .await
        .expect_err("ended sandbox");
    assert_eq!(error.code(), "job.sandbox_ended");
    running.stop().await;
}

#[tokio::test]
async fn sandboxes_need_a_registered_snapshot() {
    let running = start();
    let spec = SandboxSpec::builder()
        .snapshot(SnapshotId::from(Digest::from_blake3([1; 32])))
        .build();
    let error = running
        .bus
        .dispatch(CreateSandbox { spec }, context())
        .await
        .expect_err("unknown snapshot");
    assert_eq!(error.code(), "validation.invalid");
    running.stop().await;
}

#[tokio::test]
async fn snapshots_need_their_layers_and_round_trip() {
    let running = start();
    let missing = RegisterSnapshot {
        base: None,
        layers: vec![SnapshotLayer::new(
            Digest::from_blake3([3; 32]),
            MediaType::Tar,
        )],
        env: igloo_core::process::EnvVars::default(),
    };
    let error = running
        .bus
        .dispatch(missing, context())
        .await
        .expect_err("missing layer");
    assert_eq!(error.code(), "validation.invalid");
    let id = running.snapshot().await;
    let snapshots = Snapshots::new(running.stores.blobs.clone());
    let manifest = snapshots
        .manifest(id)
        .await
        .expect("read")
        .expect("registered");
    assert_eq!(manifest.layers().len(), 1);
    running.stop().await;
}

#[tokio::test]
async fn a_snapshot_over_a_base_keeps_the_base_layers_first() {
    let running = start();
    let base = running.snapshot().await;
    let top = b"top".to_vec();
    let digest = Digest::from_blake3(*blake3::hash(&top).as_bytes());
    let data = Box::pin(std::io::Cursor::new(top));
    running
        .bus
        .dispatch(StoreBlob { digest, data }, context())
        .await
        .expect("store layer");
    let layer = SnapshotLayer::new(digest, MediaType::Tar);
    let id = running
        .bus
        .dispatch(
            RegisterSnapshot {
                base: Some(base),
                layers: vec![layer],
                env: igloo_core::process::EnvVars::default(),
            },
            context(),
        )
        .await
        .expect("register");
    let snapshots = Snapshots::new(running.stores.blobs.clone());
    let base = snapshots.manifest(base).await.expect("read").expect("base");
    let manifest = snapshots
        .manifest(id)
        .await
        .expect("read")
        .expect("registered");
    assert_eq!(manifest.layers()[..1], base.layers()[..]);
    assert_eq!(manifest.layers()[1], layer);

    let error = running
        .bus
        .dispatch(
            RegisterSnapshot {
                base: Some(SnapshotId::from(Digest::from_blake3([5; 32]))),
                layers: vec![layer],
                env: igloo_core::process::EnvVars::default(),
            },
            context(),
        )
        .await
        .expect_err("unknown base");
    assert_eq!(error.code(), "validation.invalid");
    running.stop().await;
}

#[tokio::test]
async fn imported_images_keep_their_layers_in_order() {
    let running = start();
    let importer = ImageImporter::new(
        Arc::clone(&running.registry),
        Arc::clone(&running.stores.blobs),
    );
    let image = FIXTURE_IMAGE.parse().expect("reference");
    let platform = ImageRegistryConformance::platform();
    let (layers, env) = importer.import(&image, &platform).await.expect("import");
    assert_eq!(
        env.iter().collect::<Vec<_>>(),
        [("PATH", "/opt/tool/bin:/usr/bin:/bin")]
    );
    let expected: Vec<_> = ImageRegistryConformance::layers()
        .into_iter()
        .map(|(media_type, bytes)| {
            SnapshotLayer::new(
                Digest::from_blake3(*blake3::hash(&bytes).as_bytes()),
                media_type,
            )
        })
        .collect();
    assert_eq!(layers, expected);
    let again = importer
        .import(&image, &platform)
        .await
        .expect("import again")
        .0;
    assert_eq!(again, expected);

    let missing = "registry.test/igloo/missing:1".parse().expect("reference");
    let error = importer
        .import(&missing, &platform)
        .await
        .expect_err("missing");
    assert_eq!(error.code(), "image.not_found");
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_seal_registers_the_base_layers_plus_the_sealed_one_or_fails_with_its_sandbox() {
    let running = start();
    running.register_worker().await;
    let sandbox = running.create_sandbox().await;
    running.wait_until_placed(sandbox).await;
    let error = running
        .bus
        .dispatch(CreateSeal { sandbox }, context())
        .await
        .expect_err("not running yet");
    assert_eq!(error.code(), "seal.sandbox_not_running");
    running.record(sandbox, SandboxPhase::Running, 1).await;

    let seal = running.seal(sandbox).await;
    let layer = b"sealed changes".to_vec();
    let digest = Digest::from_blake3(*blake3::hash(&layer).as_bytes());
    let data = Box::pin(std::io::Cursor::new(layer));
    running
        .bus
        .dispatch(StoreBlob { digest, data }, context())
        .await
        .expect("store");
    let sealed_layer = SnapshotLayer::new(digest, MediaType::Tar);
    let snapshot = running.complete_seal(seal, sealed_layer, false).await;
    assert_eq!(
        running.seal_phase(seal).await,
        SealPhase::Sealed { snapshot }
    );
    let snapshots = Snapshots::new(running.stores.blobs.clone());
    let manifest = snapshots
        .manifest(snapshot)
        .await
        .expect("read")
        .expect("registered");
    assert_eq!(
        manifest.layers().len(),
        2,
        "the base layer, then the sealed one"
    );
    assert_eq!(manifest.layers()[1], sealed_layer);

    let full = running.seal(sandbox).await;
    let whole = running.complete_seal(full, sealed_layer, true).await;
    let manifest = snapshots
        .manifest(whole)
        .await
        .expect("read")
        .expect("registered");
    assert_eq!(
        manifest.layers(),
        [sealed_layer],
        "a whole root replaces the base"
    );

    let abandoned = running.seal(sandbox).await;
    running.record(sandbox, SandboxPhase::Stopped, 1).await;
    wait_until(async || {
        running.seal_phase(abandoned).await
            == SealPhase::Failed {
                reason: SealFailure::SandboxEnded,
            }
    })
    .await;
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn placement_prefers_the_worker_whose_sandboxes_used_the_same_layers() {
    let running = start();
    let first = running.register_worker().await;
    let second = running.register_worker_with(RuntimeKind::Oci).await;
    let snapshot = running.snapshot().await;
    let create = async |spec: SandboxSpec| {
        let sandbox = running
            .bus
            .dispatch(CreateSandbox { spec }, context())
            .await
            .expect("create sandbox");
        running.wait_until_placed(sandbox).await
    };
    let isolated = SandboxSpec::builder()
        .snapshot(snapshot)
        .isolation(Isolation::Container)
        .build();
    assert_eq!(
        create(isolated).await,
        second,
        "only the second worker can host it"
    );
    let same_layers = SandboxSpec::builder().snapshot(snapshot).build();
    assert_eq!(
        create(same_layers).await,
        second,
        "its layers are cached there"
    );

    let other = b"another tree".to_vec();
    let digest = Digest::from_blake3(*blake3::hash(&other).as_bytes());
    let data = Box::pin(std::io::Cursor::new(other));
    running
        .bus
        .dispatch(StoreBlob { digest, data }, context())
        .await
        .expect("store");
    let unrelated = running
        .bus
        .dispatch(
            RegisterSnapshot {
                base: None,
                layers: vec![SnapshotLayer::new(digest, MediaType::Tar)],
                env: igloo_core::process::EnvVars::default(),
            },
            context(),
        )
        .await
        .expect("register");
    let unrelated = SandboxSpec::builder().snapshot(unrelated).build();
    assert_eq!(create(unrelated).await, first, "no cache: the lowest id");
    running.stop().await;
}

#[tokio::test(start_paused = true)]
async fn secrets_belong_to_a_repository_and_never_reach_events() {
    let running = start();
    let origin = running.stores.forge_dir.path().join("origin.git");
    let register = || RegisterRepo {
        location: RepoLocation::Local {
            path: origin.display().to_string(),
        },
        default_branch: "main".parse().expect("branch"),
        token: None,
    };
    let repo = running
        .bus
        .dispatch(register(), context())
        .await
        .expect("register");
    let duplicate = running
        .bus
        .dispatch(register(), context())
        .await
        .expect_err("one repository per location");
    assert_eq!(duplicate.code(), "repo.already_registered");

    let name: SecretName = "API_TOKEN".parse().expect("name");
    let set = SetSecret {
        repo,
        name: name.clone(),
        value: SecretValue::try_from("hunter2".to_owned()).expect("value"),
    };
    running.bus.dispatch(set, context()).await.expect("set");
    assert_eq!(
        running
            .stores
            .secrets
            .get(repo, &name)
            .await
            .expect("get")
            .map(|value| value.expose().to_owned()),
        Some("hunter2".to_owned())
    );
    let events = running
        .stores
        .log
        .read(crate::ports::Sequence::START, 1000)
        .await
        .expect("events");
    assert!(
        events
            .iter()
            .any(|event| event.kind == "igloo.repo.secret_set")
    );
    assert!(
        events
            .iter()
            .all(|event| !event.data.to_string().contains("hunter2")),
        "no event holds a secret's value"
    );

    running.register_worker().await;
    let plain = running.create_sandbox().await;
    let spec = SandboxSpec::builder()
        .snapshot(running.snapshot().await)
        .repo(repo)
        .build();
    let with_repo = running
        .bus
        .dispatch(CreateSandbox { spec }, context())
        .await
        .expect("sandbox");
    let submit = |sandbox, secrets: &[&str]| {
        let mut spec = job_spec(sandbox);
        let JobSpec::Execute { secrets: names, .. } = &mut spec;
        names.extend(secrets.iter().map(|name| name.parse().expect("name")));
        running.bus.dispatch(SubmitJob { spec }, context())
    };
    let error = submit(plain, &["API_TOKEN"])
        .await
        .expect_err("no repository");
    assert_eq!(error.code(), "validation.invalid");
    let error = submit(with_repo, &["MISSING"]).await.expect_err("not set");
    assert_eq!(error.code(), "validation.invalid");
    submit(with_repo, &["API_TOKEN"]).await.expect("submit");

    running
        .bus
        .dispatch(DeleteSecret { repo, name }, context())
        .await
        .expect("delete");
    assert_eq!(running.stores.secrets.names(repo).await.expect("names"), []);
    running.stop().await;
}

#[tokio::test]
async fn a_checkout_over_a_warm_snapshot_deletes_removed_files_and_replaces_git() {
    let running = start();
    let dir = tempfile::tempdir().expect("dir");
    let origin = igloo_git::testing::Fixture::new(dir.path());
    let first = origin.commit(
        "change",
        &[
            ("Cargo.lock", Some("v1")),
            ("src/a.rs", Some("a1")),
            ("src/b.rs", Some("b1")),
        ],
    );
    let second = origin.commit("change", &[("src/a.rs", Some("a2")), ("src/b.rs", None)]);
    let third = origin.commit("change", &[("Cargo.lock", Some("v2"))]);
    let register = RegisterRepo {
        location: origin.location(),
        default_branch: "main".parse().expect("branch"),
        token: None,
    };
    let repo = running
        .bus
        .dispatch(register, context())
        .await
        .expect("repo");
    let forge = Arc::clone(&running.forge);
    let remote = crate::ports::Remote {
        repo,
        location: origin.location(),
        token: None,
    };
    forge
        .fetch(&remote, &"main".parse().expect("branch"))
        .await
        .expect("fetch");

    let snapshots = RepoSnapshots::new(forge, running.stores.blobs.clone());
    let recipe = WarmRecipe {
        command: "cargo build".to_owned(),
        lockfiles: vec!["Cargo.lock".to_owned()],
    };
    let key = |commit| snapshots.warm_key(repo, commit, None, &recipe);
    let warm_key = key(&first).await.expect("key");
    assert_eq!(
        key(&second).await.expect("key"),
        warm_key,
        "same lockfile, same key"
    );
    assert_ne!(key(&third).await.expect("key"), warm_key);

    let warm = snapshots
        .checkout(repo, &first, None, None)
        .await
        .expect("cold checkout");
    let warm = igloo_core::repo::WarmSnapshot {
        snapshot: warm,
        commit: first,
    };
    let reused = snapshots
        .checkout(repo, &second, None, Some(&warm))
        .await
        .expect("warm checkout");
    let store = Snapshots::new(running.stores.blobs.clone());
    let base = store
        .manifest(warm.snapshot)
        .await
        .expect("read")
        .expect("warm");
    let manifest = store.manifest(reused).await.expect("read").expect("reused");
    assert_eq!(
        manifest.layers()[..1],
        base.layers()[..],
        "built over the warm snapshot"
    );

    let names = running.layer_entries(manifest.layers()[1].digest()).await;
    for expected in [
        "workspace/src/a.rs",
        "workspace/src/.wh.b.rs",
        "workspace/.git/HEAD",
        "workspace/.git/.wh..wh..opq",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "{expected} in {names:?}"
        );
    }
    running.stop().await;
}
