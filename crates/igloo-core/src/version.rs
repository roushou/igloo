use std::fmt;

use serde::{Deserialize, Serialize};

/// The optimistic-concurrency version of a stored entity.
///
/// Invariant: starts at 0 before the first commit and increases by one per commit.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Version(u64);

impl Version {
    /// The version of an entity that has never been stored.
    pub const INITIAL: Self = Self(0);

    /// The version after one more commit.
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

impl From<u64> for Version {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The generation of a resource's spec.
///
/// Invariant: starts at 1 when the resource is created and increases by one per spec change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct Generation(u64);

/// A generation of 0, which no resource ever has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("generation starts at 1")]
pub struct ZeroGeneration;

impl Generation {
    /// The generation of a newly created resource.
    pub const INITIAL: Self = Self(1);

    /// The generation after one more spec change.
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

impl TryFrom<u64> for Generation {
    type Error = ZeroGeneration;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value == 0 {
            Err(ZeroGeneration)
        } else {
            Ok(Self(value))
        }
    }
}

impl From<Generation> for u64 {
    fn from(generation: Generation) -> Self {
        generation.0
    }
}

impl Default for Generation {
    fn default() -> Self {
        Self::INITIAL
    }
}

impl fmt::Display for Generation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
