use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use igloo_core::{Entity, Resource};
use tokio_util::sync::CancellationToken;

use super::{
    AppError, Command, CommandBus, CommandBusBuilder, CommandHandler, Controller,
    ControllerSettings, Reactor, ReactorRunner, Reconciler, TaskSupervisor,
};
use crate::ports::{
    BlobStore, Checkpoints, Clock, EntityStore, EventLog, Forge, IdGenerator, IdempotencyStore,
    ImageRegistry, LogStore, PolicyEngine, SecretStore,
};

/// A module of the platform (sandboxes, jobs, CI, ...): registers its commands, controllers
/// and reactors in one place.
pub trait Extension: Send + 'static {
    /// A unique name.
    fn name(&self) -> &'static str;

    /// Registers everything the module provides.
    fn install(self: Box<Self>, platform: &mut PlatformBuilder) -> Result<(), InstallError>;
}

/// The adapters every module shares, chosen by composition code.
#[derive(Clone)]
pub struct Ports {
    /// The clock.
    pub clock: Arc<dyn Clock>,
    /// The ID source.
    pub ids: Arc<dyn IdGenerator>,
    /// The global event log.
    pub events: Arc<dyn EventLog>,
    /// Reactor progress.
    pub checkpoints: Arc<dyn Checkpoints>,
    /// Authorization.
    pub policy: Arc<dyn PolicyEngine>,
    /// Content-addressed bytes.
    pub blobs: Arc<dyn BlobStore>,
    /// Job output.
    pub logs: Arc<dyn LogStore>,
    /// Where container images are pulled from.
    pub registry: Arc<dyn ImageRegistry>,
    /// Responses to requests made with an idempotency key.
    pub idempotency: Arc<dyn IdempotencyStore>,
    /// Repository secrets.
    pub secrets: Arc<dyn SecretStore>,
    /// Git hosts and the server's mirrors of their repositories.
    pub forge: Arc<dyn Forge>,
}

/// Why a module cannot be installed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    /// Two modules registered a handler for the same command.
    #[error("command {0} is registered twice")]
    DuplicateCommand(&'static str),
    /// Two modules share a name.
    #[error("extension {0} is installed twice")]
    DuplicateExtension(&'static str),
    /// No store was provided for an entity a module needs.
    #[error("no store provided for {0}")]
    MissingStore(&'static str),
}

/// Assembles the platform from ports, entity stores and extensions.
pub struct PlatformBuilder {
    ports: Ports,
    stores: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    bus: CommandBusBuilder,
    tasks: Vec<BackgroundTask>,
    extensions: HashSet<&'static str>,
}

/// The assembled platform: its command bus and the background work it needs running.
pub struct Platform {
    bus: CommandBus,
    tasks: Vec<BackgroundTask>,
}

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// A controller or reactor runner, ready to start.
struct BackgroundTask {
    name: &'static str,
    start: Box<dyn FnOnce(CancellationToken) -> BoxFuture<Result<(), AppError>> + Send>,
}

impl PlatformBuilder {
    /// How long a reactor waits before retrying a failed event.
    pub const REACTOR_RETRY: Duration = Duration::from_secs(1);

    /// A builder over `ports`, bounding each command by `command_timeout`.
    #[must_use]
    pub fn new(ports: Ports, command_timeout: Duration) -> Self {
        let bus = CommandBusBuilder::new(Arc::clone(&ports.policy), command_timeout);
        Self {
            ports,
            stores: HashMap::new(),
            bus,
            tasks: Vec::new(),
            extensions: HashSet::new(),
        }
    }

    /// The shared adapters.
    #[must_use]
    pub const fn ports(&self) -> &Ports {
        &self.ports
    }

    /// Provides the store for entity `E`; done by composition code before installing modules.
    pub fn provide_store<E>(&mut self, store: Arc<dyn EntityStore<E>>)
    where
        E: Entity + Send + Sync + 'static,
    {
        self.stores.insert(TypeId::of::<E>(), Box::new(store));
    }

    /// The store for entity `E`.
    pub fn store<E>(&self) -> Result<Arc<dyn EntityStore<E>>, InstallError>
    where
        E: Entity + Send + Sync + 'static,
    {
        self.stores
            .get(&TypeId::of::<E>())
            .and_then(|store| store.downcast_ref::<Arc<dyn EntityStore<E>>>())
            .cloned()
            .ok_or(InstallError::MissingStore(E::NAME))
    }

    /// A bus handle that dispatches once the platform is built.
    #[must_use]
    pub fn bus(&self) -> CommandBus {
        self.bus.handle()
    }

    /// Registers the handler for `C`.
    pub fn command<C: Command>(
        &mut self,
        handler: impl CommandHandler<C>,
    ) -> Result<(), InstallError> {
        self.bus.register(handler)
    }

    /// Adds a controller converging every `R` with `reconciler`.
    pub fn controller<R, Rec>(
        &mut self,
        reconciler: Rec,
        settings: ControllerSettings,
    ) -> Result<(), InstallError>
    where
        R: Resource + Send + Sync + 'static,
        R::Action: Send,
        Rec: Reconciler<R>,
    {
        let controller = Controller::new(
            self.store::<R>()?,
            Arc::clone(&self.ports.events),
            Arc::clone(&self.ports.clock),
            reconciler,
            settings,
        );
        self.tasks.push(BackgroundTask {
            name: R::NAME,
            start: Box::new(move |cancel| Box::pin(controller.run(cancel))),
        });
        Ok(())
    }

    /// Adds a reactor fed from the event log.
    pub fn reactor(&mut self, reactor: impl Reactor) {
        let name = reactor.name();
        let runner = ReactorRunner::new(
            Arc::new(reactor),
            Arc::clone(&self.ports.events),
            Arc::clone(&self.ports.checkpoints),
            self.bus.handle(),
            Self::REACTOR_RETRY,
        );
        self.tasks.push(BackgroundTask {
            name,
            start: Box::new(move |cancel| Box::pin(runner.run(cancel))),
        });
    }

    /// Installs a module.
    pub fn install(&mut self, extension: impl Extension) -> Result<(), InstallError> {
        let name = extension.name();
        if !self.extensions.insert(name) {
            return Err(InstallError::DuplicateExtension(name));
        }
        Box::new(extension).install(self)
    }

    /// Freezes the command bus.
    #[must_use]
    pub fn build(self) -> Platform {
        Platform {
            bus: self.bus.build(),
            tasks: self.tasks,
        }
    }
}

impl Platform {
    /// The command bus.
    #[must_use]
    pub const fn bus(&self) -> &CommandBus {
        &self.bus
    }

    /// Starts every controller and reactor under `supervisor`; returns the bus.
    pub fn start(self, supervisor: &TaskSupervisor) -> CommandBus {
        for task in self.tasks {
            supervisor.spawn(task.name, task.start);
        }
        self.bus
    }
}
