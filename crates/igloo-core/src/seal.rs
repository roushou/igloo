//! Seals: turning a running sandbox's file system into a snapshot others can fork from.

use serde::{Deserialize, Serialize};

use crate::sandbox::SandboxId;
use crate::snapshot::SnapshotId;
use crate::{Entity, ErrorCode, Event, Id, Prefixed, Timestamp};

/// Identifies a seal (`seal_...`).
pub type SealId = Id<Seal>;

/// A request to seal a sandbox into a snapshot, and its outcome.
///
/// Invariant: a seal ends once, `Sealed` or `Failed`, and never changes after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seal {
    id: SealId,
    sandbox: SandboxId,
    base: SnapshotId,
    phase: SealPhase,
    requested_at: Timestamp,
    events: Vec<SealEvent>,
}

/// Where a seal is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum SealPhase {
    /// The sandbox's worker is packing and uploading its file system.
    Pending,
    /// The snapshot is registered. Final.
    Sealed {
        /// The new snapshot.
        snapshot: SnapshotId,
    },
    /// The seal could not complete. Final.
    Failed {
        /// Why.
        reason: SealFailure,
    },
}

/// Why a seal failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SealFailure {
    /// The sandbox stopped or failed before the seal completed.
    SandboxEnded,
    /// The worker could not pack or upload the file system.
    WorkerError,
}

/// Facts about a seal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SealEvent {
    /// A seal was requested.
    Requested {
        /// Its id.
        id: SealId,
        /// The sandbox to seal.
        sandbox: SandboxId,
        /// The snapshot the sandbox started from.
        base: SnapshotId,
        /// When.
        at: Timestamp,
    },
    /// The snapshot was registered.
    Sealed {
        /// The new snapshot.
        snapshot: SnapshotId,
    },
    /// The seal failed.
    Failed {
        /// Why.
        reason: SealFailure,
    },
}

/// Why a seal change is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SealError {
    /// The seal already ended differently.
    #[error("the seal already ended")]
    Ended,
}

impl ErrorCode for SealError {
    fn code(&self) -> &'static str {
        match self {
            Self::Ended => "seal.ended",
        }
    }
}

impl Seal {
    /// A pending seal of `sandbox`, which started from `base`.
    #[must_use]
    pub fn new(id: SealId, sandbox: SandboxId, base: SnapshotId, now: Timestamp) -> Self {
        let mut seal = Self::initial(id, sandbox, base, now);
        seal.events.push(SealEvent::Requested {
            id,
            sandbox,
            base,
            at: now,
        });
        seal
    }

    /// Records the registered `snapshot`. Repeating it is a no-op; any other end is rejected.
    pub fn complete(&mut self, snapshot: SnapshotId) -> Result<(), SealError> {
        match self.phase {
            SealPhase::Pending => {
                self.record(SealEvent::Sealed { snapshot });
                Ok(())
            }
            SealPhase::Sealed { snapshot: done } if done == snapshot => Ok(()),
            _ => Err(SealError::Ended),
        }
    }

    /// Fails the seal unless it already ended.
    pub fn fail(&mut self, reason: SealFailure) {
        if self.phase == SealPhase::Pending {
            self.record(SealEvent::Failed { reason });
        }
    }

    /// The sandbox being sealed.
    #[must_use]
    pub const fn sandbox(&self) -> SandboxId {
        self.sandbox
    }

    /// The snapshot the sandbox started from: a sealed layer goes on top of its layers.
    #[must_use]
    pub const fn base(&self) -> SnapshotId {
        self.base
    }

    /// Where the seal is.
    #[must_use]
    pub const fn phase(&self) -> SealPhase {
        self.phase
    }

    /// When it was requested.
    #[must_use]
    pub const fn requested_at(&self) -> Timestamp {
        self.requested_at
    }

    const fn initial(id: SealId, sandbox: SandboxId, base: SnapshotId, at: Timestamp) -> Self {
        Self {
            id,
            sandbox,
            base,
            phase: SealPhase::Pending,
            requested_at: at,
            events: Vec::new(),
        }
    }

    fn record(&mut self, event: SealEvent) {
        self.apply(&event);
        self.events.push(event);
    }
}

impl Prefixed for Seal {
    const PREFIX: &'static str = "seal";
}

impl Event for SealEvent {
    const SCHEMA_VERSION: u16 = 1;

    fn kind(&self) -> &'static str {
        match self {
            Self::Requested { .. } => "igloo.seal.requested",
            Self::Sealed { .. } => "igloo.seal.sealed",
            Self::Failed { .. } => "igloo.seal.failed",
        }
    }
}

impl Entity for Seal {
    const NAME: &'static str = "seal";

    type Event = SealEvent;

    fn id(&self) -> SealId {
        self.id
    }

    fn from_created(event: &SealEvent) -> Option<Self> {
        let SealEvent::Requested {
            id,
            sandbox,
            base,
            at,
        } = event
        else {
            return None;
        };
        Some(Self::initial(*id, *sandbox, *base, *at))
    }

    fn apply(&mut self, event: &SealEvent) {
        match event {
            SealEvent::Requested { .. } => {}
            SealEvent::Sealed { snapshot } => {
                self.phase = SealPhase::Sealed {
                    snapshot: *snapshot,
                };
            }
            SealEvent::Failed { reason } => self.phase = SealPhase::Failed { reason: *reason },
        }
    }

    fn take_events(&mut self) -> Vec<SealEvent> {
        std::mem::take(&mut self.events)
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::Digest;
    use crate::testing::Scenario;

    type S = Scenario<Seal>;

    fn snapshot(n: u8) -> SnapshotId {
        SnapshotId::from(Digest::from_blake3([n; 32]))
    }

    fn requested() -> SealEvent {
        SealEvent::Requested {
            id: S::ID,
            sandbox: Id::from_uuid(Uuid::from_u128(9)),
            base: snapshot(1),
            at: S::NOW,
        }
    }

    #[test]
    fn a_requested_seal_is_pending() {
        let scenario =
            S::create(|now| Seal::new(S::ID, Id::from_uuid(Uuid::from_u128(9)), snapshot(1), now))
                .then([requested()]);
        assert_eq!(scenario.state().phase(), SealPhase::Pending);
    }

    #[test]
    fn completing_records_the_snapshot_once() {
        S::given([requested()])
            .try_when(|seal, _| seal.complete(snapshot(2)))
            .then([SealEvent::Sealed {
                snapshot: snapshot(2),
            }])
            .try_when(|seal, _| seal.complete(snapshot(2)))
            .then_no_events()
            .try_when(|seal, _| seal.complete(snapshot(3)))
            .then_error("seal.ended");
    }

    #[test]
    fn a_failed_seal_stays_failed() {
        S::given([requested()])
            .when(|seal, _| seal.fail(SealFailure::SandboxEnded))
            .then([SealEvent::Failed {
                reason: SealFailure::SandboxEnded,
            }])
            .when(|seal, _| seal.fail(SealFailure::WorkerError))
            .then_no_events()
            .try_when(|seal, _| seal.complete(snapshot(2)))
            .then_error("seal.ended");
    }

    #[test]
    fn events_serialize_stably() {
        let events = vec![
            requested(),
            SealEvent::Sealed {
                snapshot: snapshot(2),
            },
            SealEvent::Failed {
                reason: SealFailure::WorkerError,
            },
        ];
        insta::assert_json_snapshot!(events);
    }
}
