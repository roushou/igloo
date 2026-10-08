use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use igloo_core::sandbox::{
    FailureReason, Sandbox, SandboxAction, SandboxError, SandboxId, SandboxPhase, SandboxSpec,
};
use igloo_core::snapshot::{SnapshotId, SnapshotLayer};
use igloo_core::worker::{Worker, WorkerEvent, WorkerId};
use igloo_core::{
    Actor, Digest, Entity, ErrorCode, Generation, Labels, Resource, SystemComponent,
    ValidationErrors,
};

use super::Snapshots;
use crate::app::{
    AppError, Command, CommandBus, CommandHandler, ControllerSettings, EntityHandler, Extension,
    InstallError, PlatformBuilder, Reactor, Reconciler, RequestContext,
};
use crate::ports::{
    Clock, EntityStore, EventEnvelope, IdGenerator, IdGeneratorExt, StorageError, Versioned,
};

/// Creates a sandbox.
pub struct CreateSandbox {
    /// Its desired state; must be `Running`.
    pub spec: SandboxSpec,
}

/// Stops a sandbox.
pub struct StopSandbox {
    /// The sandbox.
    pub sandbox: SandboxId,
}

/// Assigns a pending sandbox to a worker.
pub struct ScheduleSandbox {
    /// The sandbox.
    pub sandbox: SandboxId,
    /// The worker.
    pub worker: WorkerId,
}

/// Records the phase a worker observed.
pub struct RecordSandboxStatus {
    /// The sandbox.
    pub sandbox: SandboxId,
    /// The observed phase.
    pub phase: SandboxPhase,
    /// The spec generation the worker acted on.
    pub observed_generation: Generation,
}

impl Command for CreateSandbox {
    type Output = SandboxId;
    const NAME: &'static str = "sandbox.create";
}

impl Command for StopSandbox {
    type Output = ();
    const NAME: &'static str = "sandbox.stop";
}

impl Command for ScheduleSandbox {
    type Output = ();
    const NAME: &'static str = "sandbox.schedule";
}

impl Command for RecordSandboxStatus {
    type Output = ();
    const NAME: &'static str = "sandbox.record_status";
}

/// Creates a sandbox once its snapshot is known to exist.
struct CreateSandboxHandler {
    store: Arc<dyn EntityStore<Sandbox>>,
    snapshots: Snapshots,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
}

#[async_trait]
impl CommandHandler<CreateSandbox> for CreateSandboxHandler {
    async fn handle(
        &self,
        command: CreateSandbox,
        context: &RequestContext,
    ) -> Result<SandboxId, AppError> {
        let snapshot = command.spec.snapshot();
        if !self.snapshots.exists(snapshot).await? {
            return Err(
                ValidationErrors::single("snapshot", format!("no snapshot {snapshot}")).into(),
            );
        }
        let id = self.ids.next::<Sandbox>();
        let now = self.clock.now();
        let sandbox =
            Sandbox::new(id, command.spec, now).map_err(|error| AppError::domain(&error))?;
        self.store
            .commit(&mut Versioned::new(sandbox), &context.commit_meta(now))
            .await?;
        Ok(id)
    }
}

/// Read access to sandboxes beyond loading one by id.
#[derive(Clone)]
pub struct SandboxQueries {
    store: Arc<dyn EntityStore<Sandbox>>,
}

impl SandboxQueries {
    /// Queries over `store`.
    #[must_use]
    pub fn new(store: Arc<dyn EntityStore<Sandbox>>) -> Self {
        Self { store }
    }

    /// The sandbox `id`, if it exists.
    pub async fn get(&self, id: SandboxId) -> Result<Option<Sandbox>, StorageError> {
        Ok(self.store.load(id).await?.map(Versioned::into_inner))
    }

    /// The sandboxes assigned to `worker` that have not ended: the worker's assignment.
    pub async fn on_worker(&self, worker: WorkerId) -> Result<Vec<Sandbox>, StorageError> {
        let all = self.store.all().await?;
        Ok(all
            .into_iter()
            .filter(|sandbox| {
                sandbox.status().worker() == Some(worker) && !sandbox.status().phase().is_terminal()
            })
            .collect())
    }

    /// The sandboxes carrying every label in `filter`, ordered by id.
    pub async fn matching(&self, filter: &Labels) -> Result<Vec<Sandbox>, StorageError> {
        let mut matching: Vec<Sandbox> = self
            .store
            .all()
            .await?
            .into_iter()
            .filter(|sandbox| sandbox.labels().matches(filter))
            .collect();
        matching.sort_by_key(Entity::id);
        Ok(matching)
    }
}

/// Why a sandbox cannot be placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlacementError {
    /// No connected, schedulable worker meets the sandbox's requirements.
    #[error("no capable worker is available")]
    NoCapableWorker,
}

impl ErrorCode for PlacementError {
    fn code(&self) -> &'static str {
        match self {
            Self::NoCapableWorker => "sandbox.no_capable_worker",
        }
    }
}

/// Places pending sandboxes on a schedulable worker that meets their requirements, preferring
/// the one most likely to have their layers cached: the worker whose sandboxes used the most of
/// them. Ties go to the lowest worker id. Without a capable worker, the reconcile fails and the
/// controller retries with backoff.
struct Placer {
    bus: CommandBus,
    workers: Arc<dyn EntityStore<Worker>>,
    sandboxes: Arc<dyn EntityStore<Sandbox>>,
    snapshots: Snapshots,
    /// Layers per snapshot; snapshots never change.
    layers: Mutex<HashMap<SnapshotId, Arc<[Digest]>>>,
    ids: Arc<dyn IdGenerator>,
}

impl Reconciler<Sandbox> for Placer {
    async fn reconcile(
        &self,
        sandbox: &Sandbox,
        actions: Vec<SandboxAction>,
    ) -> Result<(), AppError> {
        for action in actions {
            match action {
                SandboxAction::Place => self.place(sandbox).await?,
            }
        }
        Ok(())
    }
}

impl Placer {
    /// The layers of every worker's sandboxes, past and present.
    async fn cached_layers(&self) -> Result<HashMap<WorkerId, HashSet<Digest>>, AppError> {
        let mut cached: HashMap<WorkerId, HashSet<Digest>> = HashMap::new();
        for sandbox in self.sandboxes.all().await? {
            if let Some(worker) = sandbox.status().worker() {
                let layers = self.layers_of(sandbox.spec().snapshot()).await?;
                cached
                    .entry(worker)
                    .or_default()
                    .extend(layers.iter().copied());
            }
        }
        Ok(cached)
    }

    async fn layers_of(&self, snapshot: SnapshotId) -> Result<Arc<[Digest]>, AppError> {
        if let Some(layers) = self
            .layers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&snapshot)
        {
            return Ok(Arc::clone(layers));
        }
        let layers: Arc<[Digest]> = match self.snapshots.manifest(snapshot).await? {
            Some(manifest) => manifest
                .layers()
                .iter()
                .map(SnapshotLayer::digest)
                .collect(),
            None => Arc::from([]),
        };
        self.layers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(snapshot, Arc::clone(&layers));
        Ok(layers)
    }

    async fn place(&self, sandbox: &Sandbox) -> Result<(), AppError> {
        let requirements = sandbox.requirements();
        let mut workers = self.workers.all().await?;
        workers.sort_by_key(Entity::id);
        let capable: Vec<&Worker> = workers
            .iter()
            .filter(|worker| {
                worker.is_schedulable() && worker.capabilities().satisfies(&requirements).is_ok()
            })
            .collect();
        let wanted = self.layers_of(sandbox.spec().snapshot()).await?;
        let cached = self.cached_layers().await?;
        let mut best: Option<(&Worker, usize)> = None;
        for worker in capable {
            let score = cached.get(&worker.id()).map_or(0, |layers| {
                wanted.iter().filter(|layer| layers.contains(layer)).count()
            });
            if best.is_none_or(|(_, top)| score > top) {
                best = Some((worker, score));
            }
        }
        let (worker, _) = best.ok_or_else(|| AppError::domain(&PlacementError::NoCapableWorker))?;
        let context = RequestContext::new(
            Actor::System {
                component: SystemComponent::Controller,
            },
            self.ids.next(),
        );
        self.bus
            .dispatch(
                ScheduleSandbox {
                    sandbox: sandbox.id(),
                    worker: worker.id(),
                },
                context,
            )
            .await
    }
}

/// Fails every unfinished sandbox of a worker once the worker is lost.
struct FailSandboxesOfLostWorkers {
    queries: SandboxQueries,
}

#[async_trait]
impl Reactor for FailSandboxesOfLostWorkers {
    fn name(&self) -> &'static str {
        "sandbox.fail_on_worker_lost"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let Some(worker) = event.subject_as::<Worker>() else {
            return Ok(());
        };
        if !matches!(event.decode::<WorkerEvent>()?, WorkerEvent::MarkedLost) {
            return Ok(());
        }
        let actor = Actor::System {
            component: SystemComponent::Reactor,
        };
        for sandbox in self.queries.on_worker(worker).await? {
            let command = RecordSandboxStatus {
                sandbox: sandbox.id(),
                phase: SandboxPhase::Failed {
                    reason: FailureReason::WorkerLost,
                },
                observed_generation: sandbox.generation(),
            };
            bus.dispatch(command, RequestContext::caused_by(event, actor))
                .await?;
        }
        Ok(())
    }
}

/// Sandboxes: lifecycle commands, placement, and failure when their worker is lost.
pub struct SandboxModule {
    /// Pacing of the placement controller.
    pub settings: ControllerSettings,
}

impl Extension for SandboxModule {
    fn name(&self) -> &'static str {
        "sandbox"
    }

    fn install(self: Box<Self>, platform: &mut PlatformBuilder) -> Result<(), InstallError> {
        let store = platform.store::<Sandbox>()?;
        let ports = platform.ports().clone();
        platform.command(CreateSandboxHandler {
            store: Arc::clone(&store),
            snapshots: Snapshots::new(Arc::clone(&ports.blobs)),
            clock: Arc::clone(&ports.clock),
            ids: Arc::clone(&ports.ids),
        })?;
        platform.command(EntityHandler::infallible(
            Arc::clone(&store),
            Arc::clone(&ports.clock),
            |command: &StopSandbox| command.sandbox,
            |sandbox: &mut Sandbox, _: &StopSandbox, _| sandbox.stop(),
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(&store),
            Arc::clone(&ports.clock),
            |command: &ScheduleSandbox| command.sandbox,
            |sandbox: &mut Sandbox, command: &ScheduleSandbox, _| -> Result<(), SandboxError> {
                sandbox.schedule(command.worker)
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(&store),
            Arc::clone(&ports.clock),
            |command: &RecordSandboxStatus| command.sandbox,
            |sandbox: &mut Sandbox, command: &RecordSandboxStatus, _| -> Result<(), SandboxError> {
                sandbox.record_status(command.phase, command.observed_generation)
            },
        ))?;
        let placer = Placer {
            bus: platform.bus(),
            workers: platform.store::<Worker>()?,
            sandboxes: Arc::clone(&store),
            snapshots: Snapshots::new(Arc::clone(&ports.blobs)),
            layers: Mutex::new(HashMap::new()),
            ids: Arc::clone(&ports.ids),
        };
        platform.controller(placer, self.settings)?;
        platform.reactor(FailSandboxesOfLostWorkers {
            queries: SandboxQueries::new(store),
        });
        Ok(())
    }
}
