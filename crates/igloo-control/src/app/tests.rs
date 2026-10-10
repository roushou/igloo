//! Acceptance tests of the application layer against the memory adapters.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use igloo_core::sandbox::{Sandbox, SandboxAction, SandboxError, SandboxSpec};
use igloo_core::snapshot::SnapshotId;
use igloo_core::worker::{
    Arch, Capabilities, Connection, Os, ProtocolVersion, RuntimeKind, Worker, WorkerAction,
    WorkerError, WorkerId,
};
use igloo_core::{Actor, Digest, ErrorCode, Id, Labels, SystemComponent, Timestamp};
use igloo_core::{Entity, Resource};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::*;
use crate::adapters::AllowAllPolicy;
use crate::adapters::memory::{
    FixedClock, MemoryBlobStore, MemoryCheckpoints, MemoryEntityStore, MemoryEventLog,
    MemoryIdempotencyStore, MemoryLogStore, MemoryRegistry, MemorySecretStore,
    SequentialIdGenerator,
};
use crate::ports::conformance::{
    BlobStoreConformance, CheckpointsConformance, EntityStoreConformance,
    IdempotencyStoreConformance, ImageRegistryConformance, LogStoreConformance,
    SecretStoreConformance,
};
use crate::ports::{
    Authorization, Denied, EventEnvelope, EventLog, PolicyEngine, Sequence, Versioned,
};
use crate::testing::{MemoryPlatform, START, context, wait_until};

struct DenyAll;

impl PolicyEngine for DenyAll {
    fn authorize(&self, _request: &Authorization<'_>) -> Result<(), Denied> {
        Err(Denied::new("nobody may do anything"))
    }
}

fn spec() -> SandboxSpec {
    SandboxSpec::builder()
        .snapshot(SnapshotId::from(Digest::from_blake3([1; 32])))
        .build()
}

fn worker_id() -> WorkerId {
    Id::from_uuid(Uuid::from_u128(500))
}

fn capabilities() -> Capabilities {
    Capabilities::new(
        Os::Linux,
        Arch::X86_64,
        [RuntimeKind::Process].into(),
        ProtocolVersion::V1,
    )
    .expect("one runtime")
}

struct CreateSandbox(SandboxSpec);

impl Command for CreateSandbox {
    type Output = Id<Sandbox>;
    const NAME: &'static str = "sandbox.create";
}

struct StopSandbox(Id<Sandbox>);

impl Command for StopSandbox {
    type Output = ();
    const NAME: &'static str = "sandbox.stop";
}

struct ScheduleSandbox(Id<Sandbox>, WorkerId);

impl Command for ScheduleSandbox {
    type Output = ();
    const NAME: &'static str = "sandbox.schedule";
}

struct MarkWorkerLost(WorkerId);

impl Command for MarkWorkerLost {
    type Output = ();
    const NAME: &'static str = "worker.mark_lost";
}

/// Registers the sandbox commands the tests use.
struct Sandboxes;

impl Extension for Sandboxes {
    fn name(&self) -> &'static str {
        "sandboxes"
    }

    fn install(self: Box<Self>, platform: &mut PlatformBuilder) -> Result<(), InstallError> {
        let store = platform.store::<Sandbox>()?;
        let clock = Arc::clone(&platform.ports().clock);
        let ids = Arc::clone(&platform.ports().ids);
        platform.command(CreateHandler::new(
            Arc::clone(&store),
            Arc::clone(&clock),
            ids,
            |id, command: &CreateSandbox, now| Sandbox::new(id, command.0.clone(), now),
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(&store),
            Arc::clone(&clock),
            |command: &StopSandbox| command.0,
            |sandbox: &mut Sandbox, _: &StopSandbox, _| sandbox.stop(),
        ))?;
        platform.command(EntityHandler::new(
            store,
            clock,
            |command: &ScheduleSandbox| command.0,
            |sandbox: &mut Sandbox, command: &ScheduleSandbox, _| -> Result<(), SandboxError> {
                sandbox.schedule(command.1)
            },
        ))
    }
}

async fn create(bus: &CommandBus) -> Id<Sandbox> {
    bus.dispatch(CreateSandbox(spec()), context())
        .await
        .expect("create")
}

async fn error_code<C: Command>(bus: &CommandBus, command: C) -> String {
    match bus.dispatch(command, context()).await {
        Ok(_) => String::from("ok"),
        Err(error) => error.code().to_owned(),
    }
}

#[tokio::test]
async fn commands_round_trip_through_the_bus() {
    let mut fixture = MemoryPlatform::new();
    fixture.builder.install(Sandboxes).expect("install");
    let platform = fixture.builder.build();
    let id = create(platform.bus()).await;
    platform
        .bus()
        .dispatch(StopSandbox(id), context())
        .await
        .expect("stop");
    let sandbox = fixture
        .stores
        .sandboxes
        .load(id)
        .await
        .expect("load")
        .expect("exists");
    assert_eq!(
        sandbox.entity().spec().desired(),
        igloo_core::sandbox::DesiredState::Stopped
    );
    let events = fixture
        .stores
        .log
        .read(Sequence::START, 10)
        .await
        .expect("read");
    assert_eq!(events.len(), 3);
    assert!(
        events
            .iter()
            .all(|event| event.correlation_id == context().correlation_id)
    );
}

#[tokio::test]
async fn an_unregistered_command_is_a_typed_error() {
    let platform = MemoryPlatform::new().builder.build();
    assert_eq!(
        error_code(platform.bus(), StopSandbox(Id::from_uuid(Uuid::nil()))).await,
        "command.unregistered"
    );
}

#[tokio::test]
async fn an_unknown_entity_is_not_found() {
    let mut fixture = MemoryPlatform::new();
    fixture.builder.install(Sandboxes).expect("install");
    let platform = fixture.builder.build();
    assert_eq!(
        error_code(platform.bus(), StopSandbox(Id::from_uuid(Uuid::nil()))).await,
        "sandbox.not_found"
    );
}

#[tokio::test]
async fn domain_rejections_keep_their_code() {
    let mut fixture = MemoryPlatform::new();
    fixture.builder.install(Sandboxes).expect("install");
    let platform = fixture.builder.build();
    let id = create(platform.bus()).await;
    let other = Id::from_uuid(Uuid::from_u128(501));
    platform
        .bus()
        .dispatch(ScheduleSandbox(id, worker_id()), context())
        .await
        .expect("schedule");
    assert_eq!(
        error_code(platform.bus(), ScheduleSandbox(id, other)).await,
        "sandbox.invalid_transition"
    );
}

#[tokio::test]
async fn a_denied_command_is_forbidden() {
    let mut fixture = MemoryPlatform::with(Arc::new(DenyAll), CommandBusBuilder::DEFAULT_TIMEOUT);
    fixture.builder.install(Sandboxes).expect("install");
    let platform = fixture.builder.build();
    assert_eq!(
        error_code(platform.bus(), CreateSandbox(spec())).await,
        "auth.forbidden"
    );
}

struct Slow;

#[async_trait]
impl CommandHandler<StopSandbox> for Slow {
    async fn handle(&self, _: StopSandbox, _: &RequestContext) -> Result<(), AppError> {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn a_slow_command_times_out() {
    let mut fixture = MemoryPlatform::with(Arc::new(AllowAllPolicy), Duration::from_secs(1));
    fixture
        .builder
        .command::<StopSandbox>(Slow)
        .expect("register");
    let platform = fixture.builder.build();
    assert_eq!(
        error_code(platform.bus(), StopSandbox(Id::from_uuid(Uuid::nil()))).await,
        "command.timeout"
    );
}

struct Upload;

impl Command for Upload {
    type Output = ();
    const NAME: &'static str = "test.upload";
    const TIMEOUT: Option<Duration> = Some(Duration::from_secs(120));
}

#[async_trait]
impl CommandHandler<Upload> for Slow {
    async fn handle(&self, _: Upload, _: &RequestContext) -> Result<(), AppError> {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn a_command_with_its_own_limit_outlives_the_bus_limit() {
    let mut fixture = MemoryPlatform::with(Arc::new(AllowAllPolicy), Duration::from_secs(1));
    fixture.builder.command::<Upload>(Slow).expect("register");
    let platform = fixture.builder.build();
    platform
        .bus()
        .dispatch(Upload, context())
        .await
        .expect("within its own limit");
}

#[test]
fn installing_twice_is_rejected() {
    let mut fixture = MemoryPlatform::new();
    fixture.builder.install(Sandboxes).expect("install");
    assert_eq!(
        fixture.builder.install(Sandboxes),
        Err(InstallError::DuplicateExtension("sandboxes"))
    );
}

/// Places every pending sandbox on one worker, failing the first `failures` attempts.
struct Placer {
    bus: CommandBus,
    failures: Mutex<u32>,
    calls: Arc<Mutex<Vec<Instant>>>,
}

impl Reconciler<Sandbox> for Placer {
    async fn reconcile(
        &self,
        sandbox: &Sandbox,
        actions: Vec<SandboxAction>,
    ) -> Result<(), AppError> {
        self.calls.lock().expect("calls").push(Instant::now());
        {
            let mut failures = self.failures.lock().expect("failures");
            if *failures > 0 {
                *failures -= 1;
                return Err(AppError::Conflict);
            }
        }
        for action in actions {
            if action == SandboxAction::Place {
                let context = RequestContext::new(
                    Actor::System {
                        component: SystemComponent::Controller,
                    },
                    Id::from_uuid(Uuid::from_u128(9)),
                );
                self.bus
                    .dispatch(ScheduleSandbox(sandbox.id(), worker_id()), context)
                    .await?;
            }
        }
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn a_controller_converges_and_backs_off_on_errors() {
    let mut fixture = MemoryPlatform::new();
    fixture.builder.install(Sandboxes).expect("install");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let placer = Placer {
        bus: fixture.builder.bus(),
        failures: Mutex::new(2),
        calls: Arc::clone(&calls),
    };
    fixture
        .builder
        .controller(placer, ControllerSettings::default())
        .expect("controller");
    let supervisor = TaskSupervisor::new();
    let bus = fixture.builder.build().start(&supervisor);
    let id = create(&bus).await;
    let sandboxes = Arc::clone(&fixture.stores.sandboxes);
    wait_until(async || {
        let sandbox = sandboxes.load(id).await.expect("load").expect("exists");
        sandbox.entity().status().worker() == Some(worker_id())
    })
    .await;
    let calls = calls.lock().expect("calls").clone();
    assert_eq!(calls.len(), 3, "two failures, then success");
    let gaps: Vec<u64> = calls
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).as_secs())
        .collect();
    assert_eq!(gaps, [1, 2], "exponential backoff");
    supervisor
        .shutdown(Duration::from_secs(1))
        .await
        .expect("shutdown");
}

/// Marks lost workers lost.
struct Reaper {
    bus: CommandBus,
    calls: Arc<Mutex<Vec<Instant>>>,
}

impl Reconciler<Worker> for Reaper {
    async fn reconcile(&self, worker: &Worker, actions: Vec<WorkerAction>) -> Result<(), AppError> {
        self.calls.lock().expect("calls").push(Instant::now());
        for action in actions {
            match action {
                WorkerAction::MarkLost => {
                    let context = RequestContext::new(
                        Actor::System {
                            component: SystemComponent::Controller,
                        },
                        Id::from_uuid(Uuid::from_u128(10)),
                    );
                    self.bus
                        .dispatch(MarkWorkerLost(worker.id()), context)
                        .await?;
                }
            }
        }
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn a_controller_rechecks_when_the_plan_asks() {
    let mut fixture = MemoryPlatform::new();
    let store = Arc::clone(&fixture.stores.workers);
    let clock = Arc::clone(&fixture.builder.ports().clock);
    fixture
        .builder
        .command(EntityHandler::new(
            store,
            clock,
            |command: &MarkWorkerLost| command.0,
            |worker: &mut Worker, _: &MarkWorkerLost, _| -> Result<(), WorkerError> {
                worker.mark_lost()
            },
        ))
        .expect("register");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let reaper = Reaper {
        bus: fixture.builder.bus(),
        calls: Arc::clone(&calls),
    };
    fixture
        .builder
        .controller(reaper, ControllerSettings::default())
        .expect("controller");
    let started = Instant::now();
    let mut worker = Worker::new(worker_id(), capabilities(), Labels::default());
    worker.disconnect(START);
    let mut worker = Versioned::new(worker);
    fixture
        .stores
        .workers
        .commit(&mut worker, &context().commit_meta(START))
        .await
        .expect("register");
    let supervisor = TaskSupervisor::new();
    let _bus = fixture.builder.build().start(&supervisor);
    let workers = Arc::clone(&fixture.stores.workers);
    wait_until(async || {
        let worker = workers
            .load(worker_id())
            .await
            .expect("load")
            .expect("exists");
        *worker.entity().status() == Connection::Lost
    })
    .await;
    let calls = calls.lock().expect("calls").clone();
    assert_eq!(
        calls.len(),
        1,
        "planned without acting until the grace period ended"
    );
    assert_eq!(
        (calls[0] - started).as_secs(),
        Worker::RECONNECT_GRACE.as_secs().unsigned_abs()
    );
    supervisor
        .shutdown(Duration::from_secs(1))
        .await
        .expect("shutdown");
}

struct Counter {
    seen: Arc<Mutex<Vec<Sequence>>>,
}

#[async_trait]
impl Reactor for Counter {
    fn name(&self) -> &'static str {
        "counter"
    }

    async fn react(&self, event: &EventEnvelope, _bus: &CommandBus) -> Result<(), AppError> {
        self.seen.lock().expect("seen").push(event.sequence);
        Ok(())
    }
}

#[tokio::test]
async fn a_reactor_processes_each_event_once() {
    let mut fixture = MemoryPlatform::new();
    fixture.builder.install(Sandboxes).expect("install");
    let checkpoints = Arc::new(MemoryCheckpoints::default());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let platform = fixture.builder.build();
    let runner = ReactorRunner::new(
        Arc::new(Counter {
            seen: Arc::clone(&seen),
        }),
        fixture.stores.log.clone(),
        checkpoints,
        platform.bus().clone(),
        Duration::from_secs(1),
    );
    let id = create(platform.bus()).await;
    assert_eq!(runner.catch_up().await.expect("first pass"), 1);
    assert_eq!(runner.catch_up().await.expect("redelivery"), 0);
    platform
        .bus()
        .dispatch(StopSandbox(id), context())
        .await
        .expect("stop");
    assert_eq!(runner.catch_up().await.expect("new events"), 2);
    assert_eq!(seen.lock().expect("seen").len(), 3);
}

#[tokio::test]
async fn the_supervisor_stops_tasks_on_shutdown() {
    let supervisor = TaskSupervisor::new();
    supervisor.spawn("waiter", |cancel: CancellationToken| async move {
        cancel.cancelled().await;
        Ok(())
    });
    assert_eq!(supervisor.shutdown(Duration::from_secs(1)).await, Ok(()));
}

#[tokio::test(start_paused = true)]
async fn the_supervisor_aborts_tasks_that_ignore_shutdown() {
    let supervisor = TaskSupervisor::new();
    supervisor.spawn("stubborn", |_| async {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        Ok(())
    });
    assert_eq!(
        supervisor.shutdown(Duration::from_secs(1)).await,
        Err(ShutdownError(1))
    );
}

#[tokio::test]
async fn memory_adapters_pass_their_conformance_suites() {
    let log = MemoryEventLog::new();
    let ids = Arc::new(SequentialIdGenerator::default());
    EntityStoreConformance {
        store: Arc::new(MemoryEntityStore::<Sandbox>::new(log.clone(), ids)),
        events: log,
    }
    .run_all()
    .await;
    CheckpointsConformance {
        checkpoints: Arc::new(MemoryCheckpoints::default()),
    }
    .run_all()
    .await;
    BlobStoreConformance {
        blobs: Arc::new(MemoryBlobStore::new(Arc::new(FixedClock::new(
            Timestamp::new(jiff::Timestamp::constant(1_767_225_600, 0)),
        )))),
    }
    .run_all()
    .await;
    LogStoreConformance {
        logs: Arc::new(MemoryLogStore::default()),
    }
    .run_all()
    .await;
    IdempotencyStoreConformance {
        store: Arc::new(MemoryIdempotencyStore::default()),
    }
    .run_all()
    .await;
    SecretStoreConformance {
        store: Arc::new(MemorySecretStore::default()),
    }
    .run_all()
    .await;
    let image = "registry.test/igloo/fixture:1".parse().expect("reference");
    ImageRegistryConformance {
        registry: Arc::new(MemoryRegistry::default().with_image(
            &image,
            ImageRegistryConformance::platform(),
            ImageRegistryConformance::layers(),
            ImageRegistryConformance::env(),
        )),
        image,
    }
    .run_all()
    .await;
}

#[test]
fn only_lost_races_timeouts_and_failed_dependencies_are_transient() {
    assert!(AppError::Conflict.is_transient());
    assert!(AppError::Timeout { command: "x" }.is_transient());
    assert!(AppError::infrastructure(std::io::Error::other("down")).is_transient());
    assert!(!AppError::not_found("job", &1).is_transient());
    assert!(!AppError::Unregistered { command: "x" }.is_transient());
}
