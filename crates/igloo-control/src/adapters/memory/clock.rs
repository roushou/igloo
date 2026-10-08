use std::sync::Mutex;

use igloo_core::Timestamp;
use jiff::SignedDuration;

use crate::ports::Clock;

/// A clock that only moves when told to.
#[derive(Debug)]
pub struct FixedClock {
    now: Mutex<Timestamp>,
}

impl FixedClock {
    /// A clock stopped at `now`.
    #[must_use]
    pub const fn new(now: Timestamp) -> Self {
        Self {
            now: Mutex::new(now),
        }
    }

    /// Moves the clock to `now`.
    pub fn set(&self, now: Timestamp) {
        *self
            .now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = now;
    }

    /// Moves the clock forward by `duration`.
    pub fn advance(&self, duration: SignedDuration) {
        let mut now = self
            .now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *now = now.saturating_add(duration);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        *self
            .now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
