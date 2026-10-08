use std::sync::atomic::{AtomicU64, Ordering};

use uuid::Uuid;

use crate::ports::IdGenerator;

/// Deterministic IDs: 1, 2, 3, ... as UUIDs.
#[derive(Debug, Default)]
pub struct SequentialIdGenerator {
    last: AtomicU64,
}

impl IdGenerator for SequentialIdGenerator {
    fn next_uuid(&self) -> Uuid {
        let next = self.last.fetch_add(1, Ordering::Relaxed) + 1;
        Uuid::from_u128(u128::from(next))
    }
}
