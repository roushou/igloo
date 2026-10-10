use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use igloo_core::build::Build;
use igloo_core::repo::{Repo, RepoId};
use igloo_core::sandbox::Sandbox;
use igloo_core::seal::{Seal, SealPhase};
use igloo_core::snapshot::{SnapshotId, SnapshotLayer};
use igloo_core::workspace::Workspace;
use igloo_core::{Actor, Digest, Entity, Resource, SystemComponent, Timestamp};
use jiff::SignedDuration;
use tokio_util::sync::CancellationToken;

use super::Snapshots;
use super::{ForgetWarmSnapshots, RecordWarmUse};
use crate::app::{AppError, Command, CommandBus, InstallError, PlatformBuilder, RequestContext};
use crate::ports::{BlobStore, Clock, EntityStore, IdGenerator, IdGeneratorExt};

/// What one sweep of the blob store kept and reclaimed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sweep {
    /// When the sweep finished.
    pub at: Timestamp,
    /// Blobs left in the store.
    pub kept_blobs: u64,
    /// Their bytes.
    pub kept_bytes: u64,
    /// Blobs the sweep deleted.
    pub reclaimed_blobs: u64,
    /// Their bytes.
    pub reclaimed_bytes: u64,
}

/// What the blob store holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageUsage {
    /// Blobs stored.
    pub blobs: u64,
    /// Their bytes.
    pub bytes: u64,
    /// The latest sweep since the server started, if one has run.
    pub last_sweep: Option<Sweep>,
    /// The recorded warm and agent snapshots with their sizes.
    pub snapshots: Vec<SnapshotSize>,
}

/// A repository's recorded warm or agent snapshot and how large it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotSize {
    /// The repository that recorded it.
    pub repo: RepoId,
    /// The key it is recorded under.
    pub key: Digest,
    /// The snapshot.
    pub snapshot: SnapshotId,
    /// The sum of its layers' sizes in bytes; layers shared with other snapshots count in each.
    pub size_bytes: u64,
}

/// Reclaims blobs nothing can use any more.
///
/// A sweep deletes a blob only when no live root reaches it and it was stored more than
/// [`LayerCollector::GRACE`] ago. Live roots are the snapshots recorded on repositories (warm,
/// agent and imported), the snapshots of sandboxes that have not ended, and those of builds
/// and seals in progress, and the snapshot each workspace's last stop sealed until the workspace
/// is deleted; a root keeps its manifest and every layer it names. Snapshot
/// manifests are flat, so the layers of a base are kept through the snapshots built over it.
/// Before each sweep, a repository forgets the warm and agent snapshots it recorded and none used
/// for [`LayerCollector::RETENTION`], keeping its most recently used one and those a live sandbox
/// runs over; a forgotten key is rebuilt when next needed. A sweep that cannot read a root
/// deletes nothing.
#[derive(Clone)]
pub struct LayerCollector {
    blobs: Arc<dyn BlobStore>,
    snapshots: Snapshots,
    repos: Arc<dyn EntityStore<Repo>>,
    sandboxes: Arc<dyn EntityStore<Sandbox>>,
    builds: Arc<dyn EntityStore<Build>>,
    seals: Arc<dyn EntityStore<Seal>>,
    workspaces: Arc<dyn EntityStore<Workspace>>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
    bus: CommandBus,
    last: Arc<Mutex<Option<Sweep>>>,
}

impl LayerCollector {
    /// How long a blob is kept whether or not anything references it.
    pub const GRACE: Duration = Duration::from_hours(24);

    /// How long a recorded warm or agent snapshot may go unused before a sweep forgets it.
    pub const RETENTION: Duration = Duration::from_hours(7 * 24);

    /// How often [`Self::run`] sweeps.
    pub const INTERVAL: Duration = Duration::from_hours(1);

    /// A collector over the blobs and stores `platform` was given, recording snapshot use and
    /// forgetting through `bus`.
    pub fn new(platform: &PlatformBuilder, bus: CommandBus) -> Result<Self, InstallError> {
        let ports = platform.ports();
        Ok(Self {
            blobs: Arc::clone(&ports.blobs),
            snapshots: Snapshots::new(Arc::clone(&ports.blobs)),
            repos: platform.store::<Repo>()?,
            sandboxes: platform.store::<Sandbox>()?,
            builds: platform.store::<Build>()?,
            seals: platform.store::<Seal>()?,
            workspaces: platform.store::<Workspace>()?,
            clock: Arc::clone(&ports.clock),
            ids: Arc::clone(&ports.ids),
            bus,
            last: Arc::default(),
        })
    }

    /// Sweeps once at start and then every [`Self::INTERVAL`] until `cancel` fires. A failed
    /// sweep is logged and retried at the next interval.
    pub async fn run(self, cancel: CancellationToken) -> Result<(), AppError> {
        let mut ticks = tokio::time::interval(Self::INTERVAL);
        loop {
            tokio::select! {
                () = cancel.cancelled() => return Ok(()),
                _ = ticks.tick() => {}
            }
            match self.sweep().await {
                Ok(sweep) => tracing::info!(
                    kept_blobs = sweep.kept_blobs,
                    reclaimed_blobs = sweep.reclaimed_blobs,
                    reclaimed_bytes = sweep.reclaimed_bytes,
                    "swept the blob store"
                ),
                Err(error) => tracing::warn!(%error, "blob store sweep failed"),
            }
        }
    }

    /// Forgets the snapshots nobody uses any more, then deletes every unreachable blob stored
    /// more than [`Self::GRACE`] ago and records the sweep. Blobs are listed before roots are
    /// read, so a root recorded during the sweep is never missed for a blob the listing saw.
    pub async fn sweep(&self) -> Result<Sweep, AppError> {
        self.retain().await?;
        let listed = self.blobs.list().await?;
        let reachable = self.reachable().await?;
        let now = self.clock.now();
        let grace = SignedDuration::try_from(Self::GRACE).unwrap_or(SignedDuration::MAX);
        let mut sweep = Sweep {
            at: now,
            kept_blobs: 0,
            kept_bytes: 0,
            reclaimed_blobs: 0,
            reclaimed_bytes: 0,
        };
        for blob in listed {
            let expired = now.duration_since(blob.stored_at) > grace;
            if expired && !reachable.contains(&blob.digest) {
                self.blobs.delete(blob.digest).await?;
                sweep.reclaimed_blobs += 1;
                sweep.reclaimed_bytes += blob.size;
            } else {
                sweep.kept_blobs += 1;
                sweep.kept_bytes += blob.size;
            }
        }
        sweep.at = self.clock.now();
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = Some(sweep);
        Ok(sweep)
    }

    /// Stamps the recorded snapshots that have no recorded use with the current time, then
    /// forgets those of each repository last used more than [`Self::RETENTION`] ago, except
    /// its most recently used one and any a sandbox that has not ended runs over.
    async fn retain(&self) -> Result<(), AppError> {
        let now = self.clock.now();
        let retention = SignedDuration::try_from(Self::RETENTION).unwrap_or(SignedDuration::MAX);
        let cutoff = now.saturating_add(-retention);
        let live = self.live_sandbox_layers().await?;
        for repo in self.repos.all().await? {
            let unstamped: Vec<Digest> = repo
                .warm_snapshots()
                .filter(|(key, _)| repo.warm_last_used(key).is_none())
                .map(|(key, _)| *key)
                .collect();
            if !unstamped.is_empty() {
                self.dispatch(RecordWarmUse {
                    repo: repo.id(),
                    keys: unstamped,
                })
                .await?;
            }
            let last_used = |key: &Digest| repo.warm_last_used(key).unwrap_or(now);
            let newest = repo
                .warm_snapshots()
                .max_by_key(|(key, _)| (last_used(key), **key))
                .map(|(key, _)| *key);
            let mut stale = Vec::new();
            for (key, warm) in repo.warm_snapshots() {
                if Some(*key) != newest
                    && last_used(key) <= cutoff
                    && !self.is_under(warm.snapshot, &live).await?
                {
                    stale.push(*key);
                }
            }
            if !stale.is_empty() {
                self.dispatch(ForgetWarmSnapshots {
                    repo: repo.id(),
                    keys: stale,
                    unused_since: cutoff,
                })
                .await?;
            }
        }
        Ok(())
    }

    /// The snapshot id and layer digests of every sandbox that has not ended.
    async fn live_sandbox_layers(&self) -> Result<Vec<(SnapshotId, Vec<Digest>)>, AppError> {
        let mut live = Vec::new();
        for sandbox in self.sandboxes.all().await? {
            if sandbox.status().phase().is_terminal() {
                continue;
            }
            let id = sandbox.spec().snapshot();
            let layers = match self.snapshots.manifest(id).await? {
                Some(manifest) => manifest
                    .layers()
                    .iter()
                    .map(SnapshotLayer::digest)
                    .collect(),
                None => Vec::new(),
            };
            live.push((id, layers));
        }
        Ok(live)
    }

    /// Whether a sandbox in `live` runs over `snapshot`: it is that snapshot, or its layers
    /// begin with all of `snapshot`'s.
    async fn is_under(
        &self,
        snapshot: SnapshotId,
        live: &[(SnapshotId, Vec<Digest>)],
    ) -> Result<bool, AppError> {
        if live.is_empty() {
            return Ok(false);
        }
        let layers: Vec<Digest> = match self.snapshots.manifest(snapshot).await? {
            Some(manifest) => manifest
                .layers()
                .iter()
                .map(SnapshotLayer::digest)
                .collect(),
            None => Vec::new(),
        };
        Ok(live
            .iter()
            .any(|(id, over)| *id == snapshot || (!layers.is_empty() && over.starts_with(&layers))))
    }

    async fn dispatch<C: Command>(&self, command: C) -> Result<C::Output, AppError> {
        let context = RequestContext::new(
            Actor::System {
                component: SystemComponent::Controller,
            },
            self.ids.next(),
        );
        self.bus.dispatch(command, context).await
    }

    /// What the store holds, the latest sweep, and the size of every recorded snapshot.
    pub async fn usage(&self) -> Result<StorageUsage, AppError> {
        let listed = self.blobs.list().await?;
        let sizes: HashMap<Digest, u64> =
            listed.iter().map(|blob| (blob.digest, blob.size)).collect();
        let mut repos = self.repos.all().await?;
        repos.sort_by_key(Entity::id);
        let mut snapshots = Vec::new();
        for repo in &repos {
            for (key, warm) in repo.warm_snapshots() {
                let size_bytes = match self.snapshots.manifest(warm.snapshot).await? {
                    Some(manifest) => manifest
                        .layers()
                        .iter()
                        .map(|layer| sizes.get(&layer.digest()).copied().unwrap_or(0))
                        .sum(),
                    None => 0,
                };
                snapshots.push(SnapshotSize {
                    repo: repo.id(),
                    key: *key,
                    snapshot: warm.snapshot,
                    size_bytes,
                });
            }
        }
        Ok(StorageUsage {
            blobs: u64::try_from(listed.len()).unwrap_or(u64::MAX),
            bytes: listed.iter().map(|blob| blob.size).sum(),
            last_sweep: *self.last.lock().unwrap_or_else(PoisonError::into_inner),
            snapshots,
        })
    }

    /// The manifests and layers of every live root.
    async fn reachable(&self) -> Result<HashSet<Digest>, AppError> {
        let mut roots: HashSet<SnapshotId> = HashSet::new();
        for repo in self.repos.all().await? {
            roots.extend(repo.warm_snapshots().map(|(_, warm)| warm.snapshot));
            roots.extend(repo.image_snapshots());
        }
        roots.extend(
            self.sandboxes
                .all()
                .await?
                .iter()
                .filter(|sandbox| !sandbox.status().phase().is_terminal())
                .map(|sandbox| sandbox.spec().snapshot()),
        );
        roots.extend(
            self.builds
                .all()
                .await?
                .iter()
                .filter(|build| build.outcome().is_none())
                .map(|build| build.spec().sandbox.snapshot()),
        );
        roots.extend(
            self.workspaces
                .all()
                .await?
                .iter()
                .filter(|workspace| !workspace.is_deleted())
                .filter_map(|workspace| workspace.status().snapshot()),
        );
        roots.extend(
            self.seals
                .all()
                .await?
                .iter()
                .filter(|seal| matches!(seal.phase(), SealPhase::Pending))
                .map(Seal::base),
        );
        let mut reachable = HashSet::new();
        for root in roots {
            reachable.insert(*root.as_digest());
            if let Some(manifest) = self.snapshots.manifest(root).await? {
                reachable.extend(manifest.layers().iter().map(SnapshotLayer::digest));
            }
        }
        Ok(reachable)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use igloo_core::process::EnvVars;
    use igloo_core::repo::{RepoLocation, WarmSnapshot};
    use igloo_core::sandbox::{SandboxId, SandboxSpec};
    use igloo_core::snapshot::MediaType;

    use super::*;
    use crate::app::{CommandBus, ControllerSettings, TaskSupervisor};
    use crate::platform::{
        CreateSandbox, RecordWarmSnapshot, RecordWarmUse, RegisterRepo, RegisterSnapshot,
        RepoModule, SandboxModule, SnapshotModule, StopSandbox, StoreBlob,
    };
    use crate::ports::Versioned;
    use crate::testing::{MemoryPlatform, context};
    use igloo_core::Id;
    use igloo_core::seal::Seal as SealEntity;
    use igloo_core::workspace::{Workspace, WorkspaceId};
    use std::collections::BTreeSet;
    use uuid::Uuid;

    struct Fixture {
        bus: CommandBus,
        blobs: Arc<dyn BlobStore>,
        collector: LayerCollector,
        repo: RepoId,
        repos: Arc<dyn EntityStore<Repo>>,
        supervisor: TaskSupervisor,
        workspaces: Arc<dyn EntityStore<Workspace>>,
        ids: Arc<dyn IdGenerator>,
    }

    fn digest_of(bytes: &[u8]) -> Digest {
        Digest::from_blake3(*blake3::hash(bytes).as_bytes())
    }

    impl Fixture {
        async fn new() -> Self {
            let MemoryPlatform {
                mut builder,
                stores,
            } = MemoryPlatform::new();
            builder
                .install(SandboxModule {
                    settings: ControllerSettings::default(),
                })
                .expect("sandbox module");
            builder.install(SnapshotModule).expect("snapshot module");
            builder.install(RepoModule).expect("repo module");
            let ids = Arc::clone(&builder.ports().ids);
            let repos = builder.store::<Repo>().expect("repo store");
            let collector = LayerCollector::new(&builder, builder.bus()).expect("collector");
            let supervisor = TaskSupervisor::new();
            let bus = builder.build().start(&supervisor);
            let repo = bus
                .dispatch(
                    RegisterRepo {
                        location: RepoLocation::Local {
                            path: stores
                                .forge_dir
                                .path()
                                .join("origin.git")
                                .display()
                                .to_string(),
                        },
                        default_branch: "main".parse().expect("branch"),
                        token: None,
                    },
                    context(),
                )
                .await
                .expect("register repo");
            Self {
                bus,
                blobs: stores.blobs,
                collector,
                repo,
                repos,
                supervisor,
                workspaces: stores.workspaces,
                ids,
            }
        }

        async fn store(&self, bytes: &[u8]) -> Digest {
            let digest = digest_of(bytes);
            self.bus
                .dispatch(
                    StoreBlob {
                        digest,
                        data: Box::pin(Cursor::new(bytes.to_vec())),
                    },
                    context(),
                )
                .await
                .expect("store blob");
            digest
        }

        /// Stores one layer per entry of `contents` and registers them as a snapshot.
        async fn snapshot(&self, contents: &[&[u8]]) -> (SnapshotId, Vec<Digest>) {
            let mut layers = Vec::new();
            for bytes in contents {
                layers.push(self.store(bytes).await);
            }
            let id = self
                .bus
                .dispatch(
                    RegisterSnapshot {
                        base: None,
                        layers: layers
                            .iter()
                            .map(|digest| SnapshotLayer::new(*digest, MediaType::Tar))
                            .collect(),
                        env: EnvVars::default(),
                    },
                    context(),
                )
                .await
                .expect("register snapshot");
            (id, layers)
        }

        async fn record_warm(&self, key: u8, snapshot: SnapshotId) {
            self.bus
                .dispatch(
                    RecordWarmSnapshot {
                        repo: self.repo,
                        key: Digest::from_blake3([key; 32]),
                        warm: WarmSnapshot {
                            snapshot,
                            commit: "a".repeat(40).parse().expect("commit"),
                        },
                    },
                    context(),
                )
                .await
                .expect("record warm");
        }

        async fn use_warm(&self, keys: &[u8]) {
            self.bus
                .dispatch(
                    RecordWarmUse {
                        repo: self.repo,
                        keys: keys
                            .iter()
                            .map(|key| Digest::from_blake3([*key; 32]))
                            .collect(),
                    },
                    context(),
                )
                .await
                .expect("use warm");
        }

        async fn repo(&self) -> Repo {
            self.repos
                .load(self.repo)
                .await
                .expect("load")
                .expect("repo")
                .entity()
                .clone()
        }

        async fn sandbox(&self, snapshot: SnapshotId) -> SandboxId {
            self.bus
                .dispatch(
                    CreateSandbox {
                        spec: SandboxSpec::builder().snapshot(snapshot).build(),
                    },
                    context(),
                )
                .await
                .expect("create sandbox")
        }

        async fn delete_workspace(&self, id: WorkspaceId) {
            let mut stored = self
                .workspaces
                .load(id)
                .await
                .expect("load")
                .expect("workspace");
            stored.entity_mut().delete(crate::testing::START);
            let meta = context().commit_meta(crate::testing::START);
            self.workspaces
                .commit(&mut stored, &meta)
                .await
                .expect("commit");
        }

        /// A workspace whose last stop sealed `snapshot`.
        async fn stopped_workspace(&self, snapshot: SnapshotId) -> WorkspaceId {
            let id = self.ids.next::<Workspace>();
            let mut workspace = Workspace::new(
                id,
                Id::from_uuid(Uuid::from_u128(7)),
                self.repo,
                "main".parse().expect("branch"),
                crate::testing::START,
            );
            let sandbox = self.ids.next::<Sandbox>();
            workspace
                .sandbox_created(sandbox, BTreeSet::new())
                .expect("created");
            workspace.sandbox_running(sandbox, crate::testing::START);
            workspace.stop();
            workspace
                .sealing(self.ids.next::<SealEntity>())
                .expect("sealing");
            let seal = workspace.seal().expect("seal");
            workspace.seal_ended(seal, Ok(snapshot));
            workspace.sandbox_stopping().expect("stopping");
            workspace.sandbox_ended(sandbox);
            let meta = context().commit_meta(crate::testing::START);
            self.workspaces
                .commit(&mut Versioned::new(workspace), &meta)
                .await
                .expect("commit");
            id
        }

        async fn has(&self, digest: Digest) -> bool {
            self.blobs.contains(digest).await.expect("contains")
        }

        fn after_grace() -> Duration {
            LayerCollector::GRACE + Duration::from_secs(1)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreferenced_blob_is_deleted_after_the_grace_period_and_not_before() {
        let fixture = Fixture::new().await;
        let orphan = fixture.store(b"orphan").await;

        tokio::time::advance(Duration::from_hours(23)).await;
        let early = fixture.collector.sweep().await.expect("sweep");
        assert_eq!(early.reclaimed_blobs, 0);
        assert!(fixture.has(orphan).await, "younger than the grace period");

        tokio::time::advance(Duration::from_hours(1) + Duration::from_secs(1)).await;
        let sweep = fixture.collector.sweep().await.expect("sweep");
        assert_eq!(
            (sweep.reclaimed_blobs, sweep.reclaimed_bytes),
            (1, b"orphan".len() as u64)
        );
        assert!(!fixture.has(orphan).await);
    }

    #[tokio::test(start_paused = true)]
    async fn a_recorded_snapshot_keeps_its_manifest_and_layers() {
        let fixture = Fixture::new().await;
        let (snapshot, layers) = fixture.snapshot(&[b"base", b"deps"]).await;
        fixture.record_warm(1, snapshot).await;

        tokio::time::advance(Fixture::after_grace()).await;
        let sweep = fixture.collector.sweep().await.expect("sweep");
        assert_eq!(sweep.reclaimed_blobs, 0);
        assert_eq!(sweep.kept_blobs, 3, "two layers and the manifest");
        assert!(fixture.has(*snapshot.as_digest()).await);
        for layer in layers {
            assert!(fixture.has(layer).await);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_replaced_warm_snapshot_loses_its_layers_unless_another_root_shares_them() {
        let fixture = Fixture::new().await;
        let (old, old_layers) = fixture.snapshot(&[b"shared", b"old deps"]).await;
        let (other, _) = fixture.snapshot(&[b"shared", b"other deps"]).await;
        fixture.record_warm(1, old).await;
        fixture.record_warm(2, other).await;
        let (new, new_layers) = fixture.snapshot(&[b"new deps"]).await;
        fixture.record_warm(1, new).await;

        tokio::time::advance(Fixture::after_grace()).await;
        fixture.collector.sweep().await.expect("sweep");
        assert!(!fixture.has(*old.as_digest()).await, "old manifest");
        let [shared, old_only] = old_layers[..] else {
            panic!("two layers");
        };
        assert!(fixture.has(shared).await, "still in the other snapshot");
        assert!(!fixture.has(old_only).await);
        assert!(fixture.has(*new.as_digest()).await);
        assert!(fixture.has(new_layers[0]).await);
    }

    #[tokio::test(start_paused = true)]
    async fn a_sandbox_keeps_its_snapshot_until_it_stops() {
        let fixture = Fixture::new().await;
        let (snapshot, layers) = fixture.snapshot(&[b"sandbox layer"]).await;
        let sandbox = fixture.sandbox(snapshot).await;

        tokio::time::advance(Fixture::after_grace()).await;
        assert_eq!(
            fixture
                .collector
                .sweep()
                .await
                .expect("sweep")
                .reclaimed_blobs,
            0
        );

        fixture
            .bus
            .dispatch(StopSandbox { sandbox }, context())
            .await
            .expect("stop");
        let sweep = fixture.collector.sweep().await.expect("sweep");
        assert_eq!(sweep.reclaimed_blobs, 2, "the layer and the manifest");
        assert!(!fixture.has(layers[0]).await);
    }

    #[tokio::test(start_paused = true)]
    async fn a_snapshot_unused_for_seven_days_is_forgotten_and_its_layers_reclaimed() {
        let fixture = Fixture::new().await;
        let (stale, stale_layers) = fixture.snapshot(&[b"stale deps"]).await;
        let (used, used_layers) = fixture.snapshot(&[b"used deps"]).await;
        fixture.record_warm(1, stale).await;
        fixture.record_warm(2, used).await;
        fixture.use_warm(&[1, 2]).await;

        tokio::time::advance(LayerCollector::RETENTION + Duration::from_hours(1)).await;
        fixture.use_warm(&[2]).await;
        fixture.collector.sweep().await.expect("sweep");

        let repo = fixture.repo().await;
        assert_eq!(repo.warm(&Digest::from_blake3([1; 32])), None, "forgotten");
        assert!(repo.warm(&Digest::from_blake3([2; 32])).is_some());
        assert!(!fixture.has(*stale.as_digest()).await);
        assert!(!fixture.has(stale_layers[0]).await);
        assert!(fixture.has(*used.as_digest()).await);
        assert!(fixture.has(used_layers[0]).await);
    }

    #[tokio::test(start_paused = true)]
    async fn a_snapshot_used_within_seven_days_is_kept() {
        let fixture = Fixture::new().await;
        let (old, _) = fixture.snapshot(&[b"old"]).await;
        let (newer, _) = fixture.snapshot(&[b"newer"]).await;
        fixture.record_warm(1, old).await;
        fixture.record_warm(2, newer).await;
        fixture.use_warm(&[1]).await;

        tokio::time::advance(Duration::from_hours(7 * 24 - 1)).await;
        fixture.use_warm(&[2]).await;
        fixture.collector.sweep().await.expect("sweep");

        assert_eq!(fixture.repo().await.warm_snapshots().count(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn the_most_recently_used_snapshot_is_kept_however_old() {
        let fixture = Fixture::new().await;
        let (older, _) = fixture.snapshot(&[b"older"]).await;
        let (latest, _) = fixture.snapshot(&[b"latest"]).await;
        fixture.record_warm(1, older).await;
        fixture.record_warm(2, latest).await;
        fixture.use_warm(&[1]).await;
        tokio::time::advance(Duration::from_hours(2)).await;
        fixture.use_warm(&[2]).await;

        tokio::time::advance(LayerCollector::RETENTION * 4).await;
        fixture.collector.sweep().await.expect("sweep");

        let repo = fixture.repo().await;
        assert_eq!(repo.warm(&Digest::from_blake3([1; 32])), None);
        assert!(repo.warm(&Digest::from_blake3([2; 32])).is_some());
        assert!(fixture.has(*latest.as_digest()).await);
    }

    #[tokio::test(start_paused = true)]
    async fn a_snapshot_under_a_live_sandbox_is_kept_until_the_sandbox_ends() {
        let fixture = Fixture::new().await;
        let (under, _) = fixture.snapshot(&[b"warm deps"]).await;
        let (latest, _) = fixture.snapshot(&[b"latest"]).await;
        fixture.record_warm(1, under).await;
        fixture.record_warm(2, latest).await;
        fixture.use_warm(&[1]).await;
        let (checkout, _) = fixture.snapshot(&[b"warm deps", b"checkout"]).await;
        let sandbox = fixture.sandbox(checkout).await;

        tokio::time::advance(LayerCollector::RETENTION + Duration::from_hours(1)).await;
        fixture.use_warm(&[2]).await;
        fixture.collector.sweep().await.expect("sweep");
        assert!(
            fixture
                .repo()
                .await
                .warm(&Digest::from_blake3([1; 32]))
                .is_some(),
            "a live sandbox runs over it"
        );

        fixture
            .bus
            .dispatch(StopSandbox { sandbox }, context())
            .await
            .expect("stop");
        fixture.collector.sweep().await.expect("sweep");
        assert_eq!(
            fixture.repo().await.warm(&Digest::from_blake3([1; 32])),
            None
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_workspaces_sealed_snapshot_is_kept_until_the_workspace_is_deleted() {
        let fixture = Fixture::new().await;
        let (sealed, layers) = fixture.snapshot(&[b"checkout", b"sealed changes"]).await;
        let workspace = fixture.stopped_workspace(sealed).await;

        tokio::time::advance(Fixture::after_grace() + LayerCollector::RETENTION).await;
        let sweep = fixture.collector.sweep().await.expect("sweep");
        assert_eq!(
            sweep.reclaimed_blobs, 0,
            "a stopped workspace resumes from it"
        );
        assert!(fixture.has(*sealed.as_digest()).await);
        for layer in &layers {
            assert!(fixture.has(*layer).await);
        }

        fixture.delete_workspace(workspace).await;
        fixture.collector.sweep().await.expect("sweep");
        assert!(!fixture.has(*sealed.as_digest()).await);
        for layer in &layers {
            assert!(!fixture.has(*layer).await);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_snapshot_with_no_recorded_use_starts_its_seven_days_at_the_first_sweep() {
        let fixture = Fixture::new().await;
        let (first, _) = fixture.snapshot(&[b"first"]).await;
        let (second, _) = fixture.snapshot(&[b"second"]).await;
        fixture.record_warm(1, first).await;
        fixture.record_warm(2, second).await;

        tokio::time::advance(LayerCollector::RETENTION * 2).await;
        fixture.collector.sweep().await.expect("sweep");
        assert_eq!(fixture.repo().await.warm_snapshots().count(), 2);

        tokio::time::advance(LayerCollector::RETENTION + Duration::from_hours(1)).await;
        fixture.collector.sweep().await.expect("sweep");
        assert_eq!(
            fixture.repo().await.warm_snapshots().count(),
            1,
            "only the most recently used remains"
        );
    }
    #[tokio::test(start_paused = true)]
    async fn a_second_sweep_reclaims_nothing() {
        let fixture = Fixture::new().await;
        fixture.store(b"orphan").await;
        let (snapshot, _) = fixture.snapshot(&[b"kept"]).await;
        fixture.record_warm(1, snapshot).await;

        tokio::time::advance(Fixture::after_grace()).await;
        let first = fixture.collector.sweep().await.expect("sweep");
        let second = fixture.collector.sweep().await.expect("sweep");
        assert_eq!(first.reclaimed_blobs, 1);
        assert_eq!((second.reclaimed_blobs, second.reclaimed_bytes), (0, 0));
        assert_eq!(second.kept_blobs, first.kept_blobs);
        assert_eq!(
            fixture.collector.usage().await.expect("usage").last_sweep,
            Some(second)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn usage_sums_each_recorded_snapshots_layer_sizes() {
        let fixture = Fixture::new().await;
        let (snapshot, _) = fixture.snapshot(&[b"12345", b"1234567"]).await;
        fixture.record_warm(1, snapshot).await;
        fixture.store(b"orphan").await;

        let usage = fixture.collector.usage().await.expect("usage");
        assert_eq!(usage.blobs, 4);
        assert_eq!(usage.last_sweep, None);
        assert_eq!(
            usage.snapshots,
            vec![SnapshotSize {
                repo: fixture.repo,
                key: Digest::from_blake3([1; 32]),
                snapshot,
                size_bytes: 12,
            }]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_collector_sweeps_at_start_and_every_hour() {
        let fixture = Fixture::new().await;
        let orphan = fixture.store(b"orphan").await;
        let run = fixture.supervisor.spawner();
        let collector = fixture.collector.clone();
        run.spawn("layer-gc", |cancel| collector.run(cancel));

        tokio::time::sleep(LayerCollector::GRACE + LayerCollector::INTERVAL * 2).await;
        assert!(
            !fixture.has(orphan).await,
            "swept by the first sweep past the grace period"
        );
        assert!(
            fixture
                .collector
                .usage()
                .await
                .expect("usage")
                .last_sweep
                .is_some()
        );
    }
}
