use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::ports::{Checkpoints, Sequence, StorageError};

/// Consumer checkpoints in a map.
#[derive(Debug, Default)]
pub struct MemoryCheckpoints {
    positions: Mutex<HashMap<String, Sequence>>,
}

#[async_trait]
impl Checkpoints for MemoryCheckpoints {
    async fn load(&self, consumer: &str) -> Result<Sequence, StorageError> {
        let positions = self
            .positions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(positions.get(consumer).copied().unwrap_or(Sequence::START))
    }

    async fn save(&self, consumer: &str, position: Sequence) -> Result<(), StorageError> {
        self.positions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(consumer.to_owned(), position);
        Ok(())
    }
}
