use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Timestamp;
use crate::worker::WorkerId;

/// A worker's time-bounded claim on a job.
///
/// Invariant: every lease of a job has a strictly greater token than the one before, so a
/// result carrying an older token is recognisably stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    worker: WorkerId,
    token: FencingToken,
    expires_at: Timestamp,
}

/// Orders the leases of one job; only increases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FencingToken(u64);

impl Lease {
    pub(super) const fn new(worker: WorkerId, token: FencingToken, expires_at: Timestamp) -> Self {
        Self {
            worker,
            token,
            expires_at,
        }
    }

    /// The worker holding the lease.
    #[must_use]
    pub const fn worker(&self) -> WorkerId {
        self.worker
    }

    /// The lease's fencing token.
    #[must_use]
    pub const fn token(&self) -> FencingToken {
        self.token
    }

    /// When the lease ends unless renewed.
    #[must_use]
    pub const fn expires_at(&self) -> Timestamp {
        self.expires_at
    }

    /// Whether the lease still holds at `now`.
    #[must_use]
    pub fn is_live(&self, now: Timestamp) -> bool {
        now < self.expires_at
    }

    pub(super) const fn renewed(self, expires_at: Timestamp) -> Self {
        Self { expires_at, ..self }
    }
}

impl FencingToken {
    /// The token of a job's first lease.
    pub const FIRST: Self = Self(1);

    /// The token of the next lease.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// The raw counter.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for FencingToken {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Display for FencingToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
