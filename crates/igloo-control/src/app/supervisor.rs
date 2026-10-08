use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::AppError;

/// Owns every background task: spawns them with a shared cancellation token and shuts them
/// down within a bounded grace period.
pub struct TaskSupervisor {
    spawner: TaskSpawner,
}

/// A cloneable handle that spawns tasks under a [`TaskSupervisor`], for code that starts tasks
/// on demand, such as one per connection.
#[derive(Clone)]
pub struct TaskSpawner {
    tasks: Arc<Mutex<Option<JoinSet<()>>>>,
    cancel: CancellationToken,
}

/// Tasks were still running when the grace period ended; they were aborted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{0} tasks did not stop within the grace period")]
pub struct ShutdownError(pub usize);

impl TaskSupervisor {
    /// A supervisor with no tasks.
    #[must_use]
    pub fn new() -> Self {
        Self {
            spawner: TaskSpawner {
                tasks: Arc::new(Mutex::new(Some(JoinSet::new()))),
                cancel: CancellationToken::new(),
            },
        }
    }

    /// A handle spawning tasks under this supervisor.
    #[must_use]
    pub fn spawner(&self) -> TaskSpawner {
        self.spawner.clone()
    }

    /// Spawns `task`, handing it the token that signals shutdown. Its error is logged.
    pub fn spawn<F, Fut>(&self, name: &'static str, task: F)
    where
        F: FnOnce(CancellationToken) -> Fut,
        Fut: Future<Output = Result<(), AppError>> + Send + 'static,
    {
        self.spawner.spawn(name, task);
    }

    /// Signals shutdown and waits up to `grace` for every task; aborts the rest. Tasks spawned
    /// afterwards are dropped.
    pub async fn shutdown(self, grace: Duration) -> Result<(), ShutdownError> {
        self.spawner.cancel.cancel();
        let taken = self
            .spawner
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(mut tasks) = taken else {
            return Ok(());
        };
        let drained =
            tokio::time::timeout(grace, async { while tasks.join_next().await.is_some() {} }).await;
        if drained.is_ok() {
            Ok(())
        } else {
            let remaining = tasks.len();
            tasks.abort_all();
            Err(ShutdownError(remaining))
        }
    }
}

impl Default for TaskSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskSpawner {
    /// Spawns `task` with a token cancelled at shutdown. Its error is logged. After shutdown the
    /// task is dropped without running.
    pub fn spawn<F, Fut>(&self, name: &'static str, task: F)
    where
        F: FnOnce(CancellationToken) -> Fut,
        Fut: Future<Output = Result<(), AppError>> + Send + 'static,
    {
        let future = task(self.cancel.child_token());
        let mut tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(tasks) = tasks.as_mut() else {
            tracing::warn!(task = name, "spawned after shutdown; dropped");
            return;
        };
        while tasks.try_join_next().is_some() {}
        tasks.spawn(async move {
            if let Err(error) = future.await {
                tracing::error!(task = name, %error, "background task failed");
            }
        });
    }
}
