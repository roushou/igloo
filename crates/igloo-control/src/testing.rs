//! Shared fixtures for tests: an in-memory platform on tokio's pausable clock.

#![allow(clippy::panic, reason = "test fixtures report failures by panicking")]

use std::sync::Arc;
use std::time::Duration;

use igloo_core::change::Change;
use igloo_core::job::Job;
use igloo_core::repo::Repo;
use igloo_core::sandbox::Sandbox;
use igloo_core::seal::Seal;
use igloo_core::worker::Worker;
use igloo_core::workspace::Workspace;
use igloo_core::{Actor, Id, Timestamp};
use tokio::time::Instant;
use uuid::Uuid;

use crate::adapters::memory::{
    MemoryBlobStore, MemoryCheckpoints, MemoryEntityStore, MemoryEventLog, MemoryIdempotencyStore,
    MemoryLogStore, MemoryRegistry, MemorySecretStore, SequentialIdGenerator,
};
use crate::adapters::{AllowAllPolicy, GitForge};
use crate::app::{CommandBusBuilder, PlatformBuilder, Ports, RequestContext};
use crate::inbound::BlobUrls;
use crate::ports::conformance::ImageRegistryConformance;
use crate::ports::{BlobStore, Clock, EntityStore, LogStore, PolicyEngine, SecretStore};

/// 2026-01-01T00:00:00Z, when every test platform starts.
pub(crate) const START: Timestamp = Timestamp::new(jiff::Timestamp::constant(1_767_225_600, 0));

/// A clock following tokio's pausable time, so plans and controller timers agree.
pub(crate) struct TokioClock {
    origin: Instant,
}

impl Clock for TokioClock {
    fn now(&self) -> Timestamp {
        let elapsed = jiff::SignedDuration::try_from(self.origin.elapsed()).unwrap_or_default();
        START.saturating_add(elapsed)
    }
}

/// A platform builder over memory adapters, with a store for every platform entity.
pub(crate) struct MemoryPlatform {
    pub(crate) builder: PlatformBuilder,
    pub(crate) stores: MemoryStores,
}

/// The memory adapters behind a [`MemoryPlatform`], for inspecting state in assertions.
#[derive(Clone)]
pub(crate) struct MemoryStores {
    pub(crate) log: Arc<MemoryEventLog>,
    pub(crate) sandboxes: Arc<dyn EntityStore<Sandbox>>,
    pub(crate) workers: Arc<dyn EntityStore<Worker>>,
    pub(crate) jobs: Arc<dyn EntityStore<Job>>,
    pub(crate) seals: Arc<dyn EntityStore<Seal>>,
    pub(crate) workspaces: Arc<dyn EntityStore<Workspace>>,
    pub(crate) secrets: Arc<dyn SecretStore>,
    /// Holds the forge's mirrors for as long as the platform lives.
    pub(crate) forge_dir: Arc<tempfile::TempDir>,
    pub(crate) blobs: Arc<dyn BlobStore>,
}

impl MemoryPlatform {
    pub(crate) fn new() -> Self {
        Self::with(Arc::new(AllowAllPolicy), CommandBusBuilder::DEFAULT_TIMEOUT)
    }

    pub(crate) fn with(policy: Arc<dyn PolicyEngine>, timeout: Duration) -> Self {
        let log = MemoryEventLog::new();
        let ids = Arc::new(SequentialIdGenerator::default());
        let clock = Arc::new(TokioClock {
            origin: Instant::now(),
        });
        let blobs: Arc<dyn BlobStore> = Arc::new(MemoryBlobStore::new(clock.clone()));
        let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::default());
        let forge_dir = tempfile::tempdir().expect("forge mirrors");
        let logs: Arc<dyn LogStore> = Arc::new(MemoryLogStore::default());
        let ports = Ports {
            clock: clock.clone(),
            ids: ids.clone(),
            events: log.clone(),
            checkpoints: Arc::new(MemoryCheckpoints::default()),
            policy,
            blobs: Arc::clone(&blobs),
            logs,
            registry: Arc::new(MemoryRegistry::default().with_image(
                &FIXTURE_IMAGE.parse().expect("reference"),
                ImageRegistryConformance::platform(),
                ImageRegistryConformance::layers(),
                ImageRegistryConformance::env(),
            )),
            idempotency: Arc::new(MemoryIdempotencyStore::default()),
            secrets: Arc::clone(&secrets),
            forge: Arc::new(GitForge::new(forge_dir.path().to_path_buf())),
        };
        let sandboxes: Arc<dyn EntityStore<Sandbox>> =
            Arc::new(MemoryEntityStore::new(log.clone(), ids.clone()));
        let workers: Arc<dyn EntityStore<Worker>> =
            Arc::new(MemoryEntityStore::new(log.clone(), ids.clone()));
        let jobs: Arc<dyn EntityStore<Job>> =
            Arc::new(MemoryEntityStore::new(log.clone(), ids.clone()));
        let seals: Arc<dyn EntityStore<Seal>> =
            Arc::new(MemoryEntityStore::new(log.clone(), ids.clone()));
        let repos: Arc<dyn EntityStore<Repo>> =
            Arc::new(MemoryEntityStore::new(log.clone(), ids.clone()));
        let changes: Arc<dyn EntityStore<Change>> =
            Arc::new(MemoryEntityStore::new(log.clone(), ids.clone()));
        let builds: Arc<dyn EntityStore<igloo_core::build::Build>> =
            Arc::new(MemoryEntityStore::new(log.clone(), ids.clone()));
        let runs: Arc<dyn EntityStore<crate::ci::Run>> =
            Arc::new(MemoryEntityStore::new(log.clone(), ids.clone()));
        let outcomes: Arc<dyn EntityStore<crate::ci::Outcome>> =
            Arc::new(MemoryEntityStore::new(log.clone(), ids.clone()));
        let tasks: Arc<dyn EntityStore<crate::agents::Task>> =
            Arc::new(MemoryEntityStore::new(log.clone(), ids.clone()));
        let workspaces: Arc<dyn EntityStore<Workspace>> =
            Arc::new(MemoryEntityStore::new(log.clone(), ids));
        let mut builder = PlatformBuilder::new(ports, timeout);
        builder.provide_store(Arc::clone(&sandboxes));
        builder.provide_store(Arc::clone(&workers));
        builder.provide_store(Arc::clone(&jobs));
        builder.provide_store(Arc::clone(&seals));
        builder.provide_store(Arc::clone(&repos));
        builder.provide_store(changes);
        builder.provide_store(builds);
        builder.provide_store(runs);
        builder.provide_store(outcomes);
        builder.provide_store(tasks);
        builder.provide_store(Arc::clone(&workspaces));
        Self {
            builder,
            stores: MemoryStores {
                log,
                sandboxes,
                workers,
                jobs,
                seals,
                workspaces,
                blobs,
                secrets,
                forge_dir: Arc::new(forge_dir),
            },
        }
    }
}

/// The image every memory platform's registry serves, with the registry conformance fixture's
/// layers.
pub(crate) const FIXTURE_IMAGE: &str = "registry.test/igloo/fixture:1";

/// The secret test blob URLs are signed with.
pub(crate) const BLOB_SECRET: &str = "test-secret-test-secret-test-secret";

/// The public URL of the test server.
pub(crate) fn public_url() -> reqwest::Url {
    "http://igloo.test/".parse().expect("base")
}

/// Blob URLs under `http://igloo.test/`, timed by `clock`.
pub(crate) fn blob_urls(clock: Arc<dyn Clock>) -> BlobUrls {
    BlobUrls::new(public_url(), BLOB_SECRET.parse().expect("key"), clock)
}

/// A request from a fixed human.
pub(crate) fn context() -> RequestContext {
    RequestContext::new(
        Actor::Human {
            user: Id::from_uuid(Uuid::from_u128(7)),
        },
        Id::from_uuid(Uuid::from_u128(8)),
    )
}

/// Polls `condition` every 100 ms of (usually paused) time for up to 100 s.
pub(crate) async fn wait_until(mut condition: impl AsyncFnMut() -> bool) {
    for _ in 0..1000 {
        if condition().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("condition never held");
}

/// Uploads one layer and registers it as a snapshot.
pub(crate) async fn register_snapshot(
    bus: &crate::app::CommandBus,
) -> igloo_core::snapshot::SnapshotId {
    use igloo_core::Digest;
    use igloo_core::snapshot::{MediaType, SnapshotLayer};

    use crate::platform::{RegisterSnapshot, StoreBlob};

    let layer = b"working tree".to_vec();
    let digest = Digest::from_blake3(*blake3::hash(&layer).as_bytes());
    let data = Box::pin(std::io::Cursor::new(layer));
    bus.dispatch(StoreBlob { digest, data }, context())
        .await
        .expect("store layer");
    let register = RegisterSnapshot {
        base: None,
        layers: vec![SnapshotLayer::new(digest, MediaType::Tar)],
        env: igloo_core::process::EnvVars::default(),
    };
    bus.dispatch(register, context())
        .await
        .expect("register snapshot")
}
