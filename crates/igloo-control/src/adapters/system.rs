use igloo_core::Timestamp;
use uuid::Uuid;

use crate::ports::{Authorization, Clock, Denied, IdGenerator, PolicyEngine};

/// The real clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    #[allow(
        clippy::disallowed_methods,
        reason = "the one place that reads the system clock"
    )]
    fn now(&self) -> Timestamp {
        Timestamp::from(jiff::Timestamp::now())
    }
}

/// Production IDs: UUIDv7, sortable by creation time.
#[derive(Clone, Copy, Debug, Default)]
pub struct UuidV7IdGenerator;

impl IdGenerator for UuidV7IdGenerator {
    #[allow(
        clippy::disallowed_methods,
        reason = "the one place that generates UUIDs"
    )]
    fn next_uuid(&self) -> Uuid {
        Uuid::now_v7()
    }
}

/// Allows every command. The development policy until real authorization exists.
#[derive(Clone, Copy, Debug, Default)]
pub struct AllowAllPolicy;

impl PolicyEngine for AllowAllPolicy {
    fn authorize(&self, _request: &Authorization<'_>) -> Result<(), Denied> {
        Ok(())
    }
}
