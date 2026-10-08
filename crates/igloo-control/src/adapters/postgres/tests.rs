//! Postgres adapters against a real database in a container.

use std::sync::Arc;
use std::time::Duration;

use igloo_core::sandbox::{Sandbox, SandboxSpec};
use igloo_core::snapshot::SnapshotId;
use igloo_core::{Actor, Digest, Id, Timestamp};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};
use tokio::task::JoinSet;
use uuid::Uuid;

use super::{
    PgCheckpoints, PgDatabase, PgEntityStore, PgEventLog, PgIdempotencyStore, PgLogStore,
    PgSecretStore, SecretsKey,
};
use crate::adapters::memory::SequentialIdGenerator;
use crate::app::TaskSupervisor;
use crate::ports::conformance::{
    CheckpointsConformance, EntityStoreConformance, IdempotencyStoreConformance,
    LogStoreConformance, SecretStoreConformance,
};
use crate::ports::{
    CommitMeta, EntityStore, EventLog, IdGenerator, Sequence, StorageError, Versioned,
};

const NOW: Timestamp = Timestamp::new(jiff::Timestamp::constant(1_767_225_600, 0));

/// A migrated database that lives as long as its container.
struct TestDatabase {
    _container: ContainerAsync<Postgres>,
    database: PgDatabase,
}

impl TestDatabase {
    async fn start() -> Self {
        let container = Postgres::default()
            .with_tag("17-alpine")
            .start()
            .await
            .expect("start postgres");
        let port = container.get_host_port_ipv4(5432).await.expect("port");
        let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        let database = PgDatabase::connect(&url)
            .await
            .expect("connect and migrate");
        Self {
            _container: container,
            database,
        }
    }

    fn store(&self, ids: Arc<dyn IdGenerator>) -> PgEntityStore<Sandbox> {
        PgEntityStore::new(&self.database, ids)
    }
}

/// Always returns the same UUID, so a second event in one commit violates `events.id`.
struct ConstantIds;

impl IdGenerator for ConstantIds {
    fn next_uuid(&self) -> Uuid {
        Uuid::from_u128(1)
    }
}

fn sandbox(n: u128) -> Versioned<Sandbox> {
    let spec = SandboxSpec::builder()
        .snapshot(SnapshotId::from(Digest::from_blake3([1; 32])))
        .build();
    Versioned::new(Sandbox::new(Id::from_uuid(Uuid::from_u128(n)), spec, NOW).expect("valid"))
}

fn meta() -> CommitMeta {
    CommitMeta {
        actor: Actor::Human {
            user: Id::from_uuid(Uuid::from_u128(1)),
        },
        time: NOW,
        correlation_id: Id::from_uuid(Uuid::from_u128(2)),
        causation_id: None,
    }
}

#[tokio::test]
async fn postgres_adapters_pass_their_conformance_suites() {
    let db = TestDatabase::start().await;
    let events = PgEventLog::new(&db.database).await.expect("event log");
    let supervisor = TaskSupervisor::new();
    let follower = Arc::clone(&events);
    supervisor.spawn("event-log", move |cancel| async move {
        follower.follow(cancel).await.map_err(Into::into)
    });
    EntityStoreConformance {
        store: Arc::new(db.store(Arc::new(SequentialIdGenerator::default()))),
        events,
    }
    .run_all()
    .await;
    CheckpointsConformance {
        checkpoints: Arc::new(PgCheckpoints::new(&db.database)),
    }
    .run_all()
    .await;
    LogStoreConformance {
        logs: Arc::new(PgLogStore::new(&db.database)),
    }
    .run_all()
    .await;
    IdempotencyStoreConformance {
        store: Arc::new(PgIdempotencyStore::new(&db.database)),
    }
    .run_all()
    .await;
    let key: SecretsKey = "conformance-secrets-key-0123456789abcdef"
        .parse()
        .expect("key");
    SecretStoreConformance {
        store: Arc::new(PgSecretStore::new(&db.database, &key)),
    }
    .run_all()
    .await;
    let stored: Vec<u8> = sqlx::query_scalar("SELECT ciphertext FROM repo_secrets LIMIT 1")
        .fetch_one(db.database.pool())
        .await
        .expect("stored secret");
    assert!(
        !String::from_utf8_lossy(&stored).contains("rotated")
            && !String::from_utf8_lossy(&stored).contains("s3cr3t"),
        "secrets are encrypted at rest"
    );
    supervisor
        .shutdown(Duration::from_secs(5))
        .await
        .expect("shutdown");
}

#[tokio::test]
async fn a_failed_commit_leaves_no_state_and_no_events() {
    let db = TestDatabase::start().await;
    let store = db.store(Arc::new(ConstantIds));
    let mut sandbox = sandbox(10);
    sandbox.entity_mut().stop();
    let result = store.commit(&mut sandbox, &meta()).await;
    assert!(
        matches!(result, Err(StorageError::Backend(_))),
        "{result:?}"
    );
    let id = Id::from_uuid(Uuid::from_u128(10));
    assert!(store.load(id).await.expect("load").is_none());
    assert_eq!(store.ids().await.expect("ids").len(), 0);
    let events = PgEventLog::new(&db.database).await.expect("event log");
    assert_eq!(
        events.read(Sequence::START, 10).await.expect("read").len(),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_commits_get_gapless_sequences_in_commit_order() {
    let db = TestDatabase::start().await;
    let ids: Arc<dyn IdGenerator> = Arc::new(SequentialIdGenerator::default());
    let mut commits = JoinSet::new();
    for n in 0..32 {
        let store = db.store(Arc::clone(&ids));
        commits.spawn(async move {
            let mut sandbox = sandbox(100 + n);
            store.commit(&mut sandbox, &meta()).await
        });
    }
    while let Some(result) = commits.join_next().await {
        result.expect("join").expect("commit");
    }
    let events = PgEventLog::new(&db.database).await.expect("event log");
    let logged = events.read(Sequence::START, 100).await.expect("read");
    let sequences: Vec<u64> = logged.iter().map(|event| event.sequence.get()).collect();
    assert_eq!(sequences, (1..=32).collect::<Vec<u64>>());
    assert_eq!(*events.head().borrow(), Sequence::from(32));
}
