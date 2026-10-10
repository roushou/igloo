//! Workspaces driven through the bus over memory adapters. No worker runs: the tests report what
//! a worker would, and read the snapshots the platform registers.

use std::sync::Arc;
use std::time::Duration;

use igloo_core::repo::{RepoId, SecretName};
use igloo_core::sandbox::{DesiredState, NetworkPolicy, SandboxId, SandboxPhase};
use igloo_core::seal::SealId;
use igloo_core::snapshot::{MediaType, SnapshotId, SnapshotLayer};
use igloo_core::worker::{Arch, Capabilities, Os, ProtocolVersion, RuntimeKind};
use igloo_core::workspace::{Workspace, WorkspaceId, WorkspacePhase};
use igloo_core::{Actor, Digest, ErrorCode, Id, Labels, Resource};
use uuid::Uuid;

use super::*;
use crate::app::{CommandBus, ControllerSettings, RequestContext, TaskSupervisor};
use crate::platform::{
    CompleteSeal, JobModule, RecordSandboxStatus, RecordWarmSnapshot, RegisterRepo, RegisterWorker,
    RepoModule, RepoSnapshots, SandboxModule, SealModule, SetSecret, SnapshotModule, Snapshots,
    StoreBlob, WarmRecipe, WorkerModule,
};
use crate::ports::{Forge, SecretValue};
use crate::testing::{MemoryPlatform, MemoryStores, context, public_url};

const PIPELINE: &str = "secrets = [\"TOKEN\"]\n\n[warm]\ncommand = \"make deps\"\nlockfiles = [\"deps.lock\"]\n\n\
                        [sandbox]\nnetwork = \"deny_all\"\n\n[[checks]]\nname = \"ok\"\nrun = \"true\"\n";

struct Running {
    stores: MemoryStores,
    forge: Arc<dyn Forge>,
    bus: CommandBus,
    supervisor: TaskSupervisor,
    queries: WorkspaceQueries,
    repo: RepoId,
    origin: igloo_git::testing::Fixture,
    _dir: tempfile::TempDir,
}

/// A started platform with a worker registered and a repository with a pipeline registered.
async fn start() -> Running {
    let MemoryPlatform {
        mut builder,
        stores,
    } = MemoryPlatform::new();
    let settings = ControllerSettings::default();
    builder
        .install(SandboxModule { settings })
        .expect("sandbox");
    builder.install(WorkerModule { settings }).expect("worker");
    builder.install(JobModule { settings }).expect("job");
    builder.install(SnapshotModule).expect("snapshot");
    builder.install(SealModule).expect("seal");
    builder.install(RepoModule).expect("repo");
    builder
        .install(WorkspaceModule {
            settings,
            git_base: public_url(),
        })
        .expect("workspace");
    let forge = Arc::clone(&builder.ports().forge);
    let supervisor = TaskSupervisor::new();
    let bus = builder.build().start(&supervisor);

    let dir = tempfile::tempdir().expect("dir");
    let origin = igloo_git::testing::Fixture::new(dir.path());
    origin.commit(
        "pipeline",
        &[
            (".igloo/pipeline.toml", Some(PIPELINE)),
            ("deps.lock", Some("v1")),
            ("src.txt", Some("hello")),
        ],
    );
    let capabilities = Capabilities::new(
        Os::Linux,
        Arch::X86_64,
        [RuntimeKind::Process].into(),
        ProtocolVersion::V1,
    )
    .expect("one runtime");
    bus.dispatch(
        RegisterWorker {
            capabilities,
            labels: Labels::default(),
        },
        context(),
    )
    .await
    .expect("register worker");
    let register = RegisterRepo {
        location: origin.location(),
        default_branch: "main".parse().expect("branch"),
        token: None,
    };
    let repo = bus.dispatch(register, context()).await.expect("repo");
    let queries = WorkspaceQueries::new(Arc::clone(&stores.workspaces));
    Running {
        stores,
        forge,
        bus,
        supervisor,
        queries,
        repo,
        origin,
        _dir: dir,
    }
}

/// Polls `condition` every 10 ms of real time for up to 30 s; these tests run real git, which
/// paused time would time out.
async fn eventually(mut condition: impl AsyncFnMut() -> bool) {
    for _ in 0..3000 {
        if condition().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition never held");
}

fn person() -> RequestContext {
    context()
}

impl Running {
    async fn create(&self) -> WorkspaceId {
        let command = CreateWorkspace {
            repo: self.repo,
            branch: "main".parse().expect("branch"),
        };
        self.bus
            .dispatch(command, person())
            .await
            .expect("create workspace")
    }

    async fn workspace(&self, id: WorkspaceId) -> Workspace {
        self.stores
            .workspaces
            .load(id)
            .await
            .expect("load")
            .expect("exists")
            .into_inner()
    }

    async fn phase(&self, id: WorkspaceId) -> WorkspacePhase {
        self.workspace(id).await.status().phase()
    }

    async fn wait_for(&self, id: WorkspaceId, phase: WorkspacePhase) {
        eventually(async || self.phase(id).await == phase).await;
    }

    /// Waits for the workspace's sandbox to exist and be placed on the worker.
    async fn sandbox_of(&self, id: WorkspaceId) -> SandboxId {
        eventually(async || self.workspace(id).await.status().sandbox().is_some()).await;
        let sandbox = self
            .workspace(id)
            .await
            .status()
            .sandbox()
            .expect("sandbox");
        eventually(async || self.sandbox_phase(sandbox).await != SandboxPhase::Pending).await;
        sandbox
    }

    /// Whether the sandbox was asked to stop.
    async fn stop_requested(&self, id: SandboxId) -> bool {
        self.stores
            .sandboxes
            .load(id)
            .await
            .expect("load")
            .expect("sandbox")
            .entity()
            .spec()
            .desired()
            == DesiredState::Stopped
    }

    async fn sandbox_phase(&self, id: SandboxId) -> SandboxPhase {
        self.stores
            .sandboxes
            .load(id)
            .await
            .expect("load")
            .expect("sandbox")
            .entity()
            .status()
            .phase()
    }

    /// Reports what the worker would: the sandbox now has `phase`.
    async fn report(&self, sandbox: SandboxId, phase: SandboxPhase) {
        let generation = self
            .stores
            .sandboxes
            .load(sandbox)
            .await
            .expect("load")
            .expect("sandbox")
            .entity()
            .generation();
        let command = RecordSandboxStatus {
            sandbox,
            phase,
            observed_generation: generation,
        };
        self.bus.dispatch(command, context()).await.expect("report");
    }

    /// Opens a workspace and runs it: returns it and its sandbox.
    async fn running(&self) -> (WorkspaceId, SandboxId) {
        let id = self.create().await;
        let sandbox = self.sandbox_of(id).await;
        self.report(sandbox, SandboxPhase::Running).await;
        self.wait_for(id, WorkspacePhase::Running).await;
        (id, sandbox)
    }

    async fn snapshot_of(&self, sandbox: SandboxId) -> SnapshotId {
        self.stores
            .sandboxes
            .load(sandbox)
            .await
            .expect("load")
            .expect("sandbox")
            .entity()
            .spec()
            .snapshot()
    }

    async fn seal_of(&self, id: WorkspaceId) -> SealId {
        eventually(async || self.workspace(id).await.seal().is_some()).await;
        self.workspace(id).await.seal().expect("seal")
    }

    /// Completes `seal` with a layer holding `content`; the snapshot it registers.
    async fn complete(&self, seal: SealId, content: &[u8]) -> SnapshotId {
        let digest = Digest::from_blake3(*blake3::hash(content).as_bytes());
        let data = Box::pin(std::io::Cursor::new(content.to_vec()));
        self.bus
            .dispatch(StoreBlob { digest, data }, context())
            .await
            .expect("store layer");
        let layer = SnapshotLayer::new(digest, MediaType::Tar);
        let command = CompleteSeal {
            seal,
            layer,
            full: false,
        };
        self.bus
            .dispatch(command, context())
            .await
            .expect("complete seal")
    }

    /// Stops the workspace, seals `content` and ends its sandbox.
    async fn stop_sealing(
        &self,
        id: WorkspaceId,
        sandbox: SandboxId,
        content: &[u8],
    ) -> SnapshotId {
        self.bus
            .dispatch(StopWorkspace { workspace: id }, person())
            .await
            .expect("stop");
        let seal = self.seal_of(id).await;
        let snapshot = self.complete(seal, content).await;
        eventually(async || self.stop_requested(sandbox).await).await;
        self.report(sandbox, SandboxPhase::Stopped).await;
        self.wait_for(id, WorkspacePhase::Stopped).await;
        snapshot
    }

    async fn stop(self) {
        self.supervisor
            .shutdown(Duration::from_secs(1))
            .await
            .expect("shutdown");
    }
}

#[tokio::test]
async fn opening_starts_a_sandbox_over_the_branch_head_with_the_network_allowed() {
    let running = start().await;
    let (id, sandbox) = running.running().await;

    let workspace = running.workspace(id).await;
    assert_eq!(workspace.status().sandbox(), Some(sandbox));
    assert_eq!(workspace.status().snapshot(), None, "nothing sealed yet");
    let sandbox = running
        .stores
        .sandboxes
        .load(sandbox)
        .await
        .expect("load")
        .expect("sandbox");
    let spec = sandbox.entity().spec();
    assert_eq!(
        spec.network(),
        NetworkPolicy::AllowAll,
        "despite the pipeline"
    );
    assert_eq!(spec.repo(), Some(running.repo));
    running.stop().await;
}

#[tokio::test]
async fn what_a_stop_seals_is_what_the_next_start_resumes_from() {
    let running = start().await;
    let (id, first) = running.running().await;
    let head_snapshot = running.snapshot_of(first).await;

    let sealed = running.stop_sealing(id, first, b"my notes").await;
    let workspace = running.workspace(id).await;
    assert_eq!(workspace.status().snapshot(), Some(sealed));
    assert_eq!(workspace.status().sandbox(), None);
    assert_ne!(sealed, head_snapshot);

    running
        .bus
        .dispatch(StartWorkspace { workspace: id }, person())
        .await
        .expect("start");
    eventually(async || {
        running
            .workspace(id)
            .await
            .status()
            .sandbox()
            .is_some_and(|sandbox| sandbox != first)
    })
    .await;
    let second = running
        .workspace(id)
        .await
        .status()
        .sandbox()
        .expect("sandbox");
    assert_eq!(
        running.snapshot_of(second).await,
        sealed,
        "the new sandbox starts from the sealed layers, not from the branch head"
    );

    running.report(second, SandboxPhase::Running).await;
    running.wait_for(id, WorkspacePhase::Running).await;
    let again = running.stop_sealing(id, second, b"more notes").await;
    assert_ne!(again, sealed);
    assert_eq!(running.workspace(id).await.status().snapshot(), Some(again));
    running.stop().await;
}

#[tokio::test]
async fn a_failed_seal_keeps_the_previous_snapshot_and_still_stops_the_sandbox() {
    let running = start().await;
    let (id, sandbox) = running.running().await;
    running
        .bus
        .dispatch(StopWorkspace { workspace: id }, person())
        .await
        .expect("stop");
    let seal = running.seal_of(id).await;
    running
        .bus
        .dispatch(
            crate::platform::FailSeal {
                seal,
                reason: igloo_core::seal::SealFailure::SandboxEnded,
            },
            context(),
        )
        .await
        .expect("fail seal");
    eventually(async || running.stop_requested(sandbox).await).await;
    assert_eq!(running.phase(id).await, WorkspacePhase::Stopping);
    running.report(sandbox, SandboxPhase::Stopped).await;
    running.wait_for(id, WorkspacePhase::Stopped).await;
    assert_eq!(running.workspace(id).await.status().snapshot(), None);
    running.stop().await;
}

#[tokio::test]
async fn a_workspace_with_no_terminal_attached_for_two_hours_stops_itself() {
    let running = start().await;
    let (id, sandbox) = running.running().await;
    tokio::time::pause();

    tokio::time::sleep(Duration::from_mins(90)).await;
    running
        .bus
        .dispatch(TouchWorkspace { workspace: id }, person())
        .await
        .expect("touch");
    tokio::time::sleep(Duration::from_mins(90)).await;
    assert_eq!(
        running.phase(id).await,
        WorkspacePhase::Running,
        "a terminal 90 minutes ago keeps it"
    );

    tokio::time::sleep(Duration::from_mins(31)).await;
    running.seal_of(id).await;
    assert_eq!(running.phase(id).await, WorkspacePhase::Stopping);
    assert_eq!(
        running.workspace(id).await.status().sandbox(),
        Some(sandbox)
    );
    running.stop().await;
}

#[tokio::test]
async fn deleting_stops_the_sandbox_without_sealing_and_hides_the_workspace() {
    let running = start().await;
    let (id, sandbox) = running.running().await;

    running
        .bus
        .dispatch(DeleteWorkspace { workspace: id }, person())
        .await
        .expect("delete");
    eventually(async || running.stop_requested(sandbox).await).await;
    running.report(sandbox, SandboxPhase::Stopped).await;
    running.wait_for(id, WorkspacePhase::Stopped).await;

    assert_eq!(running.workspace(id).await.seal(), None);
    assert_eq!(running.workspace(id).await.status().snapshot(), None);
    assert!(running.queries.get(id).await.expect("get").is_none());
    assert_eq!(running.queries.all().await.expect("all").len(), 0);
    let error = running
        .bus
        .dispatch(StartWorkspace { workspace: id }, person())
        .await
        .expect_err("a deleted workspace never starts again");
    assert_eq!(error.code(), "workspace.deleted");
    running.stop().await;
}

#[tokio::test]
async fn a_workspace_needs_a_person_and_a_repository() {
    let running = start().await;
    let command = |repo| CreateWorkspace {
        repo,
        branch: "main".parse().expect("branch"),
    };
    let agent = RequestContext::new(
        Actor::System {
            component: igloo_core::SystemComponent::Controller,
        },
        Id::from_uuid(Uuid::from_u128(9)),
    );
    let error = running
        .bus
        .dispatch(command(running.repo), agent)
        .await
        .expect_err("no person");
    assert_eq!(error.code(), "validation.invalid");

    let unknown: RepoId = Id::from_uuid(Uuid::from_u128(99));
    let error = running
        .bus
        .dispatch(command(unknown), person())
        .await
        .expect_err("no repository");
    assert_eq!(error.code(), "repo.not_found");
    running.stop().await;
}

#[tokio::test]
async fn terminals_get_the_repository_secrets_the_pipeline_grants_as_they_are_now() {
    let running = start().await;
    let token: SecretName = "TOKEN".parse().expect("name");
    let set = |value: &str| SetSecret {
        repo: running.repo,
        name: token.clone(),
        value: SecretValue::try_from(value.to_owned()).expect("value"),
    };
    running
        .bus
        .dispatch(set("one"), context())
        .await
        .expect("set");
    let secrets =
        WorkspaceSecrets::new(running.queries.clone(), Arc::clone(&running.stores.secrets));
    let (id, sandbox) = running.running().await;

    let env = secrets.terminal_env(sandbox).await.expect("env");
    assert_eq!(env.get("TOKEN").map(String::as_str), Some("one"));
    running
        .bus
        .dispatch(set("two"), context())
        .await
        .expect("set");
    let env = secrets.terminal_env(sandbox).await.expect("env");
    assert_eq!(env.get("TOKEN").map(String::as_str), Some("two"));
    assert_eq!(secrets.workspace_of(sandbox).await.expect("of"), Some(id));

    let other: SandboxId = Id::from_uuid(Uuid::from_u128(77));
    assert!(secrets.terminal_env(other).await.expect("env").is_empty());
    assert_eq!(secrets.workspace_of(other).await.expect("of"), None);
    running.stop().await;
}

#[tokio::test]
async fn workspaces_of_one_branch_share_the_warm_snapshots_layers() {
    let running = start().await;
    let head = running.origin.head("main");
    let remote = crate::ports::Remote {
        repo: running.repo,
        location: running.origin.location(),
        token: None,
    };
    running
        .forge
        .fetch(&remote, &"main".parse().expect("branch"))
        .await
        .expect("fetch");

    let blobs = Arc::clone(&running.stores.blobs);
    let checkouts = RepoSnapshots::new(Arc::clone(&running.forge), Arc::clone(&blobs));
    let recipe = WarmRecipe {
        command: "make deps".to_owned(),
        lockfiles: vec!["deps.lock".to_owned()],
    };
    let key = checkouts
        .warm_key(running.repo, &head, None, &recipe)
        .await
        .expect("key");
    let built = checkouts
        .checkout(running.repo, &head, None, None)
        .await
        .expect("warm checkout");
    running
        .bus
        .dispatch(
            RecordWarmSnapshot {
                repo: running.repo,
                key,
                warm: igloo_core::repo::WarmSnapshot {
                    snapshot: built,
                    commit: head.clone(),
                },
            },
            context(),
        )
        .await
        .expect("record warm");

    let snapshots = Snapshots::new(blobs);
    let warm_layers = snapshots
        .manifest(built)
        .await
        .expect("read")
        .expect("warm")
        .layers()
        .to_vec();
    let first = running.create().await;
    let second = running.create().await;
    for workspace in [first, second] {
        let sandbox = running.sandbox_of(workspace).await;
        let snapshot = running.snapshot_of(sandbox).await;
        let layers = snapshots
            .manifest(snapshot)
            .await
            .expect("read")
            .expect("registered")
            .layers()
            .to_vec();
        assert_eq!(
            layers[..warm_layers.len()],
            warm_layers[..],
            "built over the warm snapshot's layers"
        );
    }
    running.stop().await;
}

impl Running {
    /// The jobs that set up workspaces' sandboxes, with their environment.
    async fn setup_jobs(&self) -> Vec<(SandboxId, std::collections::BTreeMap<String, String>)> {
        let mut jobs = Vec::new();
        for job in self.stores.jobs.all().await.expect("jobs") {
            let spec = job.spec();
            if !setup::WorkspaceSetup::is_setup(spec) {
                continue;
            }
            let igloo_core::job::JobSpec::Execute { env, .. } = spec;
            let env = env
                .iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect();
            jobs.push((spec.sandbox(), env));
        }
        jobs
    }
}

#[tokio::test]
async fn a_new_workspace_is_set_up_once_with_origin_at_igloo_and_the_repositorys_dotfiles() {
    let running = start().await;
    let dotfiles = igloo_core::dotfiles::Dotfiles::new(
        "https://github.com/me/dotfiles",
        "./install.sh --quiet",
    )
    .expect("dotfiles");
    running
        .bus
        .dispatch(
            crate::platform::SetDotfiles {
                repo: running.repo,
                dotfiles: Some(dotfiles),
            },
            context(),
        )
        .await
        .expect("set dotfiles");
    let (id, sandbox) = running.running().await;

    eventually(async || !running.setup_jobs().await.is_empty()).await;
    let jobs = running.setup_jobs().await;
    assert_eq!(jobs.len(), 1);
    let (job_sandbox, env) = &jobs[0];
    assert_eq!(*job_sandbox, sandbox);
    let origin = format!("http://igloo.test/git/{}.git", running.repo);
    assert_eq!(env.get("IGLOO_GIT_URL"), Some(&origin));
    assert_eq!(
        env.get("IGLOO_WORKSPACE_BRANCH").map(String::as_str),
        Some("main")
    );
    assert_eq!(
        env.get("IGLOO_DOTFILES_REPOSITORY").map(String::as_str),
        Some("https://github.com/me/dotfiles")
    );
    assert_eq!(
        env.get("IGLOO_DOTFILES_INSTALL").map(String::as_str),
        Some("./install.sh --quiet")
    );
    assert!(
        env.keys().all(|name| !name.contains("TOKEN")),
        "no credential goes into a job, which the event log records"
    );

    let sealed = running.stop_sealing(id, sandbox, b"my notes").await;
    running
        .bus
        .dispatch(StartWorkspace { workspace: id }, person())
        .await
        .expect("start");
    eventually(async || {
        running
            .workspace(id)
            .await
            .status()
            .sandbox()
            .is_some_and(|second| second != sandbox)
    })
    .await;
    let second = running
        .workspace(id)
        .await
        .status()
        .sandbox()
        .expect("sandbox");
    assert_eq!(running.snapshot_of(second).await, sealed);
    running.report(second, SandboxPhase::Running).await;
    running.wait_for(id, WorkspacePhase::Running).await;
    // The reactor handles the second run's event after the workspace records it.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        running.setup_jobs().await.len(),
        1,
        "resuming does not set up again"
    );
    running.stop().await;
}

#[tokio::test]
async fn a_repository_without_dotfiles_still_gets_its_origin_set() {
    let running = start().await;
    running.running().await;
    eventually(async || !running.setup_jobs().await.is_empty()).await;
    let jobs = running.setup_jobs().await;
    let (_, env) = &jobs[0];
    assert!(env.contains_key("IGLOO_GIT_URL"));
    assert!(!env.contains_key("IGLOO_DOTFILES_REPOSITORY"));
    assert!(!env.contains_key("IGLOO_DOTFILES_INSTALL"));
    running.stop().await;
}
