//! Seals: snapshots taken of running sandboxes.

use igloo_core::Entity;
use igloo_core::seal::{self as domain, Seal};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Where a seal is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SealPhase {
    /// The sandbox's file system is being packed and uploaded.
    Pending,
    /// The snapshot is registered; see `snapshot`.
    Sealed,
    /// The seal could not complete; see `failure_reason`.
    Failed,
}

/// A seal of a sandbox into a snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct SealResource {
    /// Its id (`seal_...`).
    pub id: String,
    /// The sealed sandbox.
    pub sandbox: String,
    /// Where it is.
    pub phase: SealPhase,
    /// The new snapshot, once sealed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    /// Why it failed: `sandbox_ended` or `worker_error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

impl From<&Seal> for SealResource {
    fn from(seal: &Seal) -> Self {
        let (phase, snapshot, failure_reason) = match seal.phase() {
            domain::SealPhase::Pending => (SealPhase::Pending, None, None),
            domain::SealPhase::Sealed { snapshot } => {
                (SealPhase::Sealed, Some(snapshot.to_string()), None)
            }
            domain::SealPhase::Failed { reason } => {
                let reason = match reason {
                    domain::SealFailure::SandboxEnded => "sandbox_ended",
                    domain::SealFailure::WorkerError => "worker_error",
                };
                (SealPhase::Failed, None, Some(reason.to_owned()))
            }
        };
        Self {
            id: seal.id().to_string(),
            sandbox: seal.sandbox().to_string(),
            phase,
            snapshot,
            failure_reason,
        }
    }
}
