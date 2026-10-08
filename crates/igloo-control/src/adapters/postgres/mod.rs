//! Postgres adapters. Events are every entity's source of truth: loading replays them, and each
//! commit appends them under one advisory lock so sequences are gapless and follow commit order.

mod checkpoints;
mod event_log;
mod idempotency;
mod logs;
mod secrets;
mod store;

#[cfg(test)]
mod tests;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::ports::StorageError;

pub use checkpoints::PgCheckpoints;
pub use event_log::PgEventLog;
pub use idempotency::PgIdempotencyStore;
pub use logs::PgLogStore;
pub use secrets::{PgSecretStore, SecretsKey, WeakSecretsKey};
pub use store::PgEntityStore;

/// A connection pool to a migrated Igloo database.
#[derive(Clone, Debug)]
pub struct PgDatabase {
    pool: PgPool,
}

impl PgDatabase {
    /// The channel notified with the latest sequence after every commit.
    pub(crate) const CHANNEL: &'static str = "igloo_events";

    /// The advisory lock key serializing commits ("igloo" in ASCII, then 1).
    pub(crate) const COMMIT_LOCK: i64 = 0x6967_6c6f_6f00_0001;

    /// Connects to `url` and applies pending migrations.
    pub async fn connect(url: &str) -> Result<Self, StorageError> {
        let pool = PgPoolOptions::new()
            .max_connections(16)
            .connect(url)
            .await
            .map_err(StorageError::backend)?;
        sqlx::migrate!()
            .run(&pool)
            .await
            .map_err(StorageError::backend)?;
        Ok(Self { pool })
    }

    /// The pool, for constructing adapters.
    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }
}
