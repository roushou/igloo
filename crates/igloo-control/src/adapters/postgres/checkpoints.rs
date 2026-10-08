use async_trait::async_trait;
use sqlx::PgPool;

use super::PgDatabase;
use crate::ports::{Checkpoints, Sequence, StorageError};

/// Consumer checkpoints in the `checkpoints` table.
pub struct PgCheckpoints {
    pool: PgPool,
}

impl PgCheckpoints {
    /// Checkpoints over `database`.
    #[must_use]
    pub fn new(database: &PgDatabase) -> Self {
        Self {
            pool: database.pool().clone(),
        }
    }
}

#[async_trait]
impl Checkpoints for PgCheckpoints {
    async fn load(&self, consumer: &str) -> Result<Sequence, StorageError> {
        let position = sqlx::query_scalar!(
            "SELECT position FROM checkpoints WHERE consumer = $1",
            consumer,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::backend)?
        .unwrap_or_default();
        Ok(Sequence::from(
            u64::try_from(position).map_err(StorageError::backend)?,
        ))
    }

    async fn save(&self, consumer: &str, position: Sequence) -> Result<(), StorageError> {
        let position = i64::try_from(position.get()).map_err(StorageError::backend)?;
        sqlx::query!(
            "INSERT INTO checkpoints (consumer, position) VALUES ($1, $2)
             ON CONFLICT (consumer) DO UPDATE SET position = EXCLUDED.position",
            consumer,
            position,
        )
        .execute(&self.pool)
        .await
        .map_err(StorageError::backend)?;
        Ok(())
    }
}
