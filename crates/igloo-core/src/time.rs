use std::fmt;
use std::str::FromStr;

use jiff::SignedDuration;
use serde::{Deserialize, Serialize};

/// A UTC instant with nanosecond precision, RFC 3339 on the wire.
///
/// There is deliberately no `now()`: time enters the domain as an argument,
/// filled from the `Clock` port.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(jiff::Timestamp);

impl Timestamp {
    /// Wraps a jiff timestamp.
    #[must_use]
    pub const fn new(inner: jiff::Timestamp) -> Self {
        Self(inner)
    }

    /// The instant `duration` after (or before, if negative) this one, clamped to the range
    /// of representable instants.
    #[must_use]
    pub fn saturating_add(self, duration: SignedDuration) -> Self {
        let bound = if duration.is_negative() {
            jiff::Timestamp::MIN
        } else {
            jiff::Timestamp::MAX
        };
        Self(self.0.saturating_add(duration).unwrap_or(bound))
    }

    /// The signed duration from `earlier` to this instant.
    #[must_use]
    pub fn duration_since(self, earlier: Self) -> SignedDuration {
        self.0.duration_since(earlier.0)
    }
}

impl From<jiff::Timestamp> for Timestamp {
    fn from(inner: jiff::Timestamp) -> Self {
        Self(inner)
    }
}

impl From<Timestamp> for jiff::Timestamp {
    fn from(timestamp: Timestamp) -> Self {
        timestamp.0
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl FromStr for Timestamp {
    type Err = jiff::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(Self)
    }
}
