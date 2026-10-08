use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use igloo_core::{Id, Plan, Resource};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::AppError;
use super::queue::{Backoff, WorkQueue};
use crate::ports::{Clock, EntityStore, EventLog, Sequence};

/// Executes the actions a resource's plan asks for.
pub trait Reconciler<R: Resource>: Send + Sync + 'static {
    /// Carries out `actions` for `resource`. Must be idempotent: the controller may call it
    /// again with the same actions after a failure or a resync.
    fn reconcile(
        &self,
        resource: &R,
        actions: Vec<R::Action>,
    ) -> impl Future<Output = Result<(), AppError>> + Send;
}

/// How a controller paces itself.
#[derive(Clone, Copy, Debug)]
pub struct ControllerSettings {
    /// How often every resource is planned again, whether or not anything changed.
    pub resync: Duration,
    /// Delays between attempts after a failure.
    pub backoff: Backoff,
}

impl Default for ControllerSettings {
    fn default() -> Self {
        Self {
            resync: Duration::from_secs(60),
            backoff: Backoff {
                initial: Duration::from_secs(1),
                max: Duration::from_secs(60),
            },
        }
    }
}

/// Converges every resource of one kind, level-triggered: plans a resource whenever one of its
/// events lands, on every resync, and when its plan asks to be rechecked; retries failures with
/// backoff. Processes one resource at a time.
pub struct Controller<R: Resource, Rec> {
    store: Arc<dyn EntityStore<R>>,
    events: Arc<dyn EventLog>,
    clock: Arc<dyn Clock>,
    reconciler: Rec,
    settings: ControllerSettings,
    queue: WorkQueue<Id<R>>,
}

impl<R, Rec> Controller<R, Rec>
where
    R: Resource + Send + Sync + 'static,
    R::Action: Send,
    Rec: Reconciler<R>,
{
    /// The most events read from the log at once.
    const PAGE: usize = 256;

    /// A controller for the resources in `store`, woken by `events`.
    pub fn new(
        store: Arc<dyn EntityStore<R>>,
        events: Arc<dyn EventLog>,
        clock: Arc<dyn Clock>,
        reconciler: Rec,
        settings: ControllerSettings,
    ) -> Self {
        Self {
            store,
            events,
            clock,
            reconciler,
            settings,
            queue: WorkQueue::new(settings.backoff),
        }
    }

    /// Runs until `cancel` fires. Starts with a resync, then follows the log from its head.
    pub async fn run(mut self, cancel: CancellationToken) -> Result<(), AppError> {
        let mut head = self.events.head();
        let mut cursor = *head.borrow_and_update();
        let mut resync = tokio::time::interval(self.settings.resync);
        loop {
            while let Some(id) = self.queue.pop(Instant::now()) {
                self.process(id).await;
            }
            let deadline = self.queue.next_deadline();
            tokio::select! {
                () = cancel.cancelled() => return Ok(()),
                _ = resync.tick() => self.resync().await,
                changed = head.changed() => {
                    if changed.is_err() {
                        return Ok(());
                    }
                    cursor = self.follow(cursor).await;
                }
                () = async {
                    match deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending().await,
                    }
                } => {}
            }
        }
    }

    /// Plans one resource and acts on the plan.
    async fn process(&mut self, id: Id<R>) {
        let resource = match self.store.load(id).await {
            Ok(Some(resource)) => resource.into_inner(),
            Ok(None) => {
                self.queue.succeeded(id);
                return;
            }
            Err(error) => {
                tracing::warn!(controller = R::NAME, %id, %error, "load failed");
                self.queue.retry(id, Instant::now());
                return;
            }
        };
        match resource.plan(self.clock.now()) {
            Plan::Converged => self.queue.succeeded(id),
            Plan::Recheck { after } => {
                self.queue.succeeded(id);
                let after = Duration::try_from(after).unwrap_or(Duration::ZERO);
                self.queue.push_at(id, Instant::now() + after);
            }
            Plan::Act(actions) => match self.reconciler.reconcile(&resource, actions).await {
                Ok(()) => self.queue.succeeded(id),
                Err(error) => {
                    tracing::warn!(controller = R::NAME, %id, %error, "reconcile failed");
                    self.queue.retry(id, Instant::now());
                }
            },
        }
    }

    /// Queues every stored resource.
    async fn resync(&mut self) {
        match self.store.ids().await {
            Ok(ids) => ids.into_iter().for_each(|id| self.queue.push(id)),
            Err(error) => tracing::warn!(controller = R::NAME, %error, "resync failed"),
        }
    }

    /// Queues the resources touched by events after `cursor`; returns the new cursor.
    async fn follow(&mut self, mut cursor: Sequence) -> Sequence {
        loop {
            let page = match self.events.read(cursor, Self::PAGE).await {
                Ok(page) => page,
                Err(error) => {
                    tracing::warn!(controller = R::NAME, %error, "reading events failed");
                    return cursor;
                }
            };
            let Some(last) = page.last() else {
                return cursor;
            };
            cursor = last.sequence;
            for event in &page {
                if let Some(id) = event.subject_as::<R>() {
                    self.queue.push(id);
                }
            }
        }
    }
}
