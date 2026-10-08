use async_trait::async_trait;
use igloo_core::{Actor, Timestamp};
use sqlx::PgPool;

use super::PgDatabase;
use crate::ports::{Claim, IdempotencyStore, KeyedRequest, StorageError, StoredResponse};

/// Idempotency records in the `idempotency_keys` table.
pub struct PgIdempotencyStore {
    pool: PgPool,
}

impl PgIdempotencyStore {
    /// Records over `database`.
    #[must_use]
    pub fn new(database: &PgDatabase) -> Self {
        Self {
            pool: database.pool().clone(),
        }
    }

    fn actor(actor: &Actor) -> Result<String, StorageError> {
        serde_json::to_string(actor).map_err(StorageError::backend)
    }

    fn time(timestamp: Timestamp) -> jiff_sqlx::Timestamp {
        jiff_sqlx::Timestamp::from(jiff::Timestamp::from(timestamp))
    }
}

#[async_trait]
impl IdempotencyStore for PgIdempotencyStore {
    async fn claim(&self, request: &KeyedRequest<'_>) -> Result<Claim, StorageError> {
        let actor = Self::actor(request.actor)?;
        let fingerprint = request.fingerprint.as_bytes().as_slice();
        loop {
            let claimed = sqlx::query_scalar!(
                "INSERT INTO idempotency_keys (actor, key, fingerprint, claimed_at)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (actor, key) DO UPDATE
                 SET fingerprint = EXCLUDED.fingerprint, claimed_at = EXCLUDED.claimed_at,
                     status = NULL, body = NULL
                 WHERE (idempotency_keys.status IS NOT NULL AND idempotency_keys.claimed_at < $5)
                    OR (idempotency_keys.status IS NULL AND idempotency_keys.claimed_at < $6)
                 RETURNING true AS \"claimed!\"",
                actor,
                request.key,
                fingerprint,
                Self::time(request.now) as _,
                Self::time(request.records_before) as _,
                Self::time(request.claims_before) as _,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(StorageError::backend)?;
            if claimed.is_some() {
                return Ok(Claim::Claimed);
            }
            let existing = sqlx::query!(
                "SELECT fingerprint, status, body FROM idempotency_keys
                 WHERE actor = $1 AND key = $2",
                actor,
                request.key,
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(StorageError::backend)?;
            // Released between the two statements: claim again.
            let Some(existing) = existing else {
                continue;
            };
            if existing.fingerprint != fingerprint {
                return Ok(Claim::Mismatch);
            }
            return Ok(match (existing.status, existing.body) {
                (Some(status), Some(body)) => Claim::Completed(StoredResponse {
                    status: u16::try_from(status).map_err(StorageError::backend)?,
                    body,
                }),
                _ => Claim::InProgress,
            });
        }
    }

    async fn complete(
        &self,
        actor: &Actor,
        key: &str,
        response: &StoredResponse,
    ) -> Result<(), StorageError> {
        sqlx::query!(
            "UPDATE idempotency_keys SET status = $3, body = $4 WHERE actor = $1 AND key = $2",
            Self::actor(actor)?,
            key,
            i16::try_from(response.status).map_err(StorageError::backend)?,
            response.body,
        )
        .execute(&self.pool)
        .await
        .map_err(StorageError::backend)?;
        Ok(())
    }

    async fn release(&self, actor: &Actor, key: &str) -> Result<(), StorageError> {
        sqlx::query!(
            "DELETE FROM idempotency_keys WHERE actor = $1 AND key = $2 AND status IS NULL",
            Self::actor(actor)?,
            key,
        )
        .execute(&self.pool)
        .await
        .map_err(StorageError::backend)?;
        Ok(())
    }
}
