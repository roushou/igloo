use serde::{Deserialize, Serialize};

use crate::Generation;
use crate::worker::WorkerId;

/// The observed state of a sandbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxStatus {
    phase: SandboxPhase,
    worker: Option<WorkerId>,
    observed_generation: Option<Generation>,
}

/// Where a sandbox is in its lifecycle.
///
/// Invariant: `Stopped` and `Failed` are terminal; a sandbox never leaves them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum SandboxPhase {
    /// Waiting for a worker.
    Pending,
    /// Assigned to a worker that has not started it yet.
    Scheduled,
    /// The worker is materializing and starting it.
    Starting,
    /// Running.
    Running,
    /// The worker is stopping it.
    Stopping,
    /// Stopped. Terminal.
    Stopped,
    /// Failed. Terminal.
    Failed {
        /// Why.
        reason: FailureReason,
    },
}

/// Why a sandbox failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    /// Its worker was lost.
    WorkerLost,
    /// Its snapshot could not be materialized.
    SnapshotUnavailable,
    /// The runtime failed to start or keep it running.
    RuntimeError,
}

impl SandboxStatus {
    /// The current phase.
    #[must_use]
    pub const fn phase(&self) -> SandboxPhase {
        self.phase
    }

    /// The worker the sandbox is assigned to, once scheduled.
    #[must_use]
    pub const fn worker(&self) -> Option<WorkerId> {
        self.worker
    }

    /// The latest spec generation the worker reported acting on.
    #[must_use]
    pub const fn observed_generation(&self) -> Option<Generation> {
        self.observed_generation
    }

    pub(super) const fn pending() -> Self {
        Self {
            phase: SandboxPhase::Pending,
            worker: None,
            observed_generation: None,
        }
    }

    pub(super) const fn scheduled(self, worker: WorkerId) -> Self {
        Self {
            phase: SandboxPhase::Scheduled,
            worker: Some(worker),
            ..self
        }
    }

    pub(super) const fn recorded(self, phase: SandboxPhase, observed: Generation) -> Self {
        Self {
            phase,
            observed_generation: Some(observed),
            ..self
        }
    }
}

impl SandboxPhase {
    /// Whether the sandbox can never change phase again.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Stopped | Self::Failed { .. })
    }

    /// A stable lowercase name for messages.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Scheduled => "scheduled",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed { .. } => "failed",
        }
    }
}
