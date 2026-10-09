use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use igloo_core::Timestamp;
use igloo_core::worker::{Usage, WorkerId};

/// The latest usage report of each worker, with the time the server received it.
///
/// Usage is telemetry, not state: it is kept in memory, never enters the event log, and is
/// empty after a restart until workers report again. Clones share the same reports.
#[derive(Clone, Default)]
pub struct WorkerUsages {
    reports: Arc<Mutex<HashMap<WorkerId, (Usage, Timestamp)>>>,
}

impl WorkerUsages {
    /// No reports.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Keeps `usage`, received at `at`, as `worker`'s latest report.
    pub fn record(&self, worker: WorkerId, usage: Usage, at: Timestamp) {
        self.reports().insert(worker, (usage, at));
    }

    /// `worker`'s latest report and when it was received, if it sent any.
    #[must_use]
    pub fn latest(&self, worker: WorkerId) -> Option<(Usage, Timestamp)> {
        self.reports().get(&worker).copied()
    }

    fn reports(&self) -> std::sync::MutexGuard<'_, HashMap<WorkerId, (Usage, Timestamp)>> {
        self.reports.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
