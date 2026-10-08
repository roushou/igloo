//! The application layer: commands and the bus, generic handlers, controllers, reactors, the
//! task supervisor, and the `Extension` seam modules plug into.

mod bus;
mod command;
mod controller;
mod error;
mod handler;
mod platform;
mod queue;
mod reactor;
mod supervisor;

#[cfg(test)]
mod tests;

pub use bus::{CommandBus, CommandBusBuilder, Dispatch};
pub use command::{Command, CommandHandler, IdempotencyKey, InvalidIdempotencyKey, RequestContext};
pub use controller::{Controller, ControllerSettings, Reconciler};
pub use error::AppError;
pub use handler::{CreateHandler, EntityHandler};
pub use platform::{Extension, InstallError, Platform, PlatformBuilder, Ports};
pub use queue::Backoff;
pub use reactor::{Reactor, ReactorRunner};
pub use supervisor::{ShutdownError, TaskSpawner, TaskSupervisor};
