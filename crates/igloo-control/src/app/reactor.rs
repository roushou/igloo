use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use super::{AppError, CommandBus};
use crate::ports::{Checkpoints, EventEnvelope, EventLog};

/// Turns events into commands.
#[async_trait]
pub trait Reactor: Send + Sync + 'static {
    /// A stable name; its checkpoint is stored under it.
    fn name(&self) -> &'static str;

    /// Reacts to one event. Called at least once per event, in log order; must be idempotent,
    /// which entity methods that ignore satisfied changes make natural.
    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError>;
}

/// Feeds a reactor every event after its checkpoint, saving the checkpoint after each one.
pub struct ReactorRunner {
    reactor: Arc<dyn Reactor>,
    events: Arc<dyn EventLog>,
    checkpoints: Arc<dyn Checkpoints>,
    bus: CommandBus,
    retry: Duration,
}

impl ReactorRunner {
    /// The most events read from the log at once.
    const PAGE: usize = 256;

    /// A runner for `reactor`, retrying a failed event after `retry`.
    pub fn new(
        reactor: Arc<dyn Reactor>,
        events: Arc<dyn EventLog>,
        checkpoints: Arc<dyn Checkpoints>,
        bus: CommandBus,
        retry: Duration,
    ) -> Self {
        Self {
            reactor,
            events,
            checkpoints,
            bus,
            retry,
        }
    }

    /// Processes every event after the checkpoint; stops at the first failure, which is
    /// retried on the next call. Returns how many events were processed.
    pub async fn catch_up(&self) -> Result<usize, AppError> {
        let name = self.reactor.name();
        let mut position = self.checkpoints.load(name).await?;
        let mut processed = 0;
        loop {
            let page = self.events.read(position, Self::PAGE).await?;
            if page.is_empty() {
                return Ok(processed);
            }
            for event in &page {
                self.reactor.react(event, &self.bus).await?;
                self.checkpoints.save(name, event.sequence).await?;
                position = event.sequence;
                processed += 1;
            }
        }
    }

    /// Runs until `cancel` fires, catching up whenever the log grows.
    pub async fn run(self, cancel: CancellationToken) -> Result<(), AppError> {
        let mut head = self.events.head();
        loop {
            head.borrow_and_update();
            let wait = match self.catch_up().await {
                Ok(_) => None,
                Err(error) => {
                    tracing::warn!(reactor = self.reactor.name(), %error, "reaction failed");
                    Some(self.retry)
                }
            };
            tokio::select! {
                () = cancel.cancelled() => return Ok(()),
                changed = head.changed(), if wait.is_none() => {
                    if changed.is_err() {
                        return Ok(());
                    }
                }
                () = tokio::time::sleep(wait.unwrap_or_default()), if wait.is_some() => {}
            }
        }
    }
}
