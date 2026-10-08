use std::sync::Arc;

use igloo_core::worker::{Capabilities, Worker, WorkerAction, WorkerError, WorkerId};
use igloo_core::{Actor, Entity, Labels, SystemComponent};

use crate::app::{
    AppError, Command, CommandBus, ControllerSettings, CreateHandler, EntityHandler, Extension,
    InstallError, PlatformBuilder, Reconciler, RequestContext,
};
use crate::ports::{IdGenerator, IdGeneratorExt};

/// Registers a worker on its first connection.
pub struct RegisterWorker {
    /// What it offers.
    pub capabilities: Capabilities,
    /// Its labels.
    pub labels: Labels,
}

/// A registered worker's stream opened.
pub struct ConnectWorker {
    /// The worker.
    pub worker: WorkerId,
    /// What it offers now.
    pub capabilities: Capabilities,
}

/// A worker's stream closed.
pub struct DisconnectWorker {
    /// The worker.
    pub worker: WorkerId,
}

/// Stops placing new work on a worker.
pub struct DrainWorker {
    /// The worker.
    pub worker: WorkerId,
}

/// Gives up on a disconnected worker.
pub struct MarkWorkerLost {
    /// The worker.
    pub worker: WorkerId,
}

impl Command for RegisterWorker {
    type Output = WorkerId;
    const NAME: &'static str = "worker.register";
}

impl Command for ConnectWorker {
    type Output = ();
    const NAME: &'static str = "worker.connect";
}

impl Command for DisconnectWorker {
    type Output = ();
    const NAME: &'static str = "worker.disconnect";
}

impl Command for DrainWorker {
    type Output = ();
    const NAME: &'static str = "worker.drain";
}

impl Command for MarkWorkerLost {
    type Output = ();
    const NAME: &'static str = "worker.mark_lost";
}

/// Marks workers lost once their reconnect grace period has passed.
struct Reaper {
    bus: CommandBus,
    ids: Arc<dyn IdGenerator>,
}

impl Reconciler<Worker> for Reaper {
    async fn reconcile(&self, worker: &Worker, actions: Vec<WorkerAction>) -> Result<(), AppError> {
        for action in actions {
            match action {
                WorkerAction::MarkLost => {
                    let context = RequestContext::new(
                        Actor::System {
                            component: SystemComponent::Controller,
                        },
                        self.ids.next(),
                    );
                    let command = MarkWorkerLost {
                        worker: worker.id(),
                    };
                    self.bus.dispatch(command, context).await?;
                }
            }
        }
        Ok(())
    }
}

/// Workers: registration, connection tracking, draining and loss.
pub struct WorkerModule {
    /// Pacing of the liveness controller.
    pub settings: ControllerSettings,
}

impl Extension for WorkerModule {
    fn name(&self) -> &'static str {
        "worker"
    }

    fn install(self: Box<Self>, platform: &mut PlatformBuilder) -> Result<(), InstallError> {
        let store = platform.store::<Worker>()?;
        let ports = platform.ports().clone();
        platform.command(CreateHandler::new(
            Arc::clone(&store),
            Arc::clone(&ports.clock),
            Arc::clone(&ports.ids),
            |id, command: &RegisterWorker, _| {
                Ok::<_, std::convert::Infallible>(Worker::new(
                    id,
                    command.capabilities.clone(),
                    command.labels.clone(),
                ))
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(&store),
            Arc::clone(&ports.clock),
            |command: &ConnectWorker| command.worker,
            |worker: &mut Worker, command: &ConnectWorker, _| -> Result<(), WorkerError> {
                worker.connect(command.capabilities.clone())
            },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(&store),
            Arc::clone(&ports.clock),
            |command: &DisconnectWorker| command.worker,
            |worker: &mut Worker, _: &DisconnectWorker, now| worker.disconnect(now),
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(&store),
            Arc::clone(&ports.clock),
            |command: &DrainWorker| command.worker,
            |worker: &mut Worker, _: &DrainWorker, _| worker.drain(),
        ))?;
        platform.command(EntityHandler::new(
            store,
            Arc::clone(&ports.clock),
            |command: &MarkWorkerLost| command.worker,
            |worker: &mut Worker, _: &MarkWorkerLost, _| -> Result<(), WorkerError> {
                worker.mark_lost()
            },
        ))?;
        let reaper = Reaper {
            bus: platform.bus(),
            ids: ports.ids,
        };
        platform.controller(reaper, self.settings)
    }
}
