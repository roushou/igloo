use async_trait::async_trait;

use super::{Sequence, StorageError};

/// How far each event consumer has processed the log, so it resumes without reprocessing.
#[async_trait]
pub trait Checkpoints: Send + Sync {
    /// The last sequence `consumer` processed, or [`Sequence::START`].
    async fn load(&self, consumer: &str) -> Result<Sequence, StorageError>;

    /// Records that `consumer` processed everything up to `position`.
    async fn save(&self, consumer: &str, position: Sequence) -> Result<(), StorageError>;
}
