use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use igloo_core::{Entity, Event, Id, Version};
use sqlx::PgPool;

use super::PgDatabase;
use crate::ports::{
    CommitMeta, EntityStore, EventMarker, IdGenerator, IdGeneratorExt, NewEvent, StorageError,
    Versioned,
};

/// Stores entities of type `E` as their event streams.
pub struct PgEntityStore<E> {
    pool: PgPool,
    ids: Arc<dyn IdGenerator>,
    _marker: PhantomData<fn() -> E>,
}

impl<E> PgEntityStore<E> {
    /// A store over `database`, naming events with `ids`.
    #[must_use]
    pub fn new(database: &PgDatabase, ids: Arc<dyn IdGenerator>) -> Self {
        Self {
            pool: database.pool().clone(),
            ids,
            _marker: PhantomData,
        }
    }
}

#[async_trait]
impl<E> EntityStore<E> for PgEntityStore<E>
where
    E: Entity + Send + Sync + 'static,
{
    async fn load(&self, id: Id<E>) -> Result<Option<Versioned<E>>, StorageError> {
        let subject = id.to_string();
        let rows = sqlx::query!(
            "SELECT schema_version, data FROM events WHERE subject = $1 ORDER BY stream_version",
            subject,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::backend)?;
        if rows.is_empty() {
            return Ok(None);
        }
        let version = Version::from(u64::try_from(rows.len()).map_err(StorageError::backend)?);
        let events = rows
            .into_iter()
            .map(|row| {
                if u16::try_from(row.schema_version).ok() != Some(E::Event::SCHEMA_VERSION) {
                    return Err(StorageError::CorruptHistory(subject.clone()));
                }
                serde_json::from_value::<E::Event>(row.data).map_err(StorageError::backend)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let entity = E::replay(events).ok_or_else(|| StorageError::CorruptHistory(subject))?;
        Ok(Some(Versioned::loaded(entity, version)))
    }

    async fn commit(
        &self,
        entity: &mut Versioned<E>,
        meta: &CommitMeta,
    ) -> Result<(), StorageError> {
        let events = entity.entity_mut().take_events();
        if events.is_empty() {
            return Ok(());
        }
        let encoded = events
            .iter()
            .map(|event| Ok((self.ids.next::<EventMarker>(), NewEvent::new(event)?)))
            .collect::<Result<Vec<_>, StorageError>>()?;
        let subject = entity.entity().id().to_string();
        let count = i64::try_from(encoded.len()).map_err(StorageError::backend)?;
        let expected = i64::try_from(entity.version().get()).map_err(StorageError::backend)?;
        let actor = serde_json::to_value(meta.actor).map_err(StorageError::backend)?;
        let time = jiff_sqlx::Timestamp::from(jiff::Timestamp::from(meta.time));
        let correlation = meta.correlation_id.to_string();
        let causation = meta.causation_id.map(|id| id.to_string());

        let mut tx = self.pool.begin().await.map_err(StorageError::backend)?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(PgDatabase::COMMIT_LOCK)
            .execute(&mut *tx)
            .await
            .map_err(StorageError::backend)?;
        let claimed = if expected == 0 {
            sqlx::query!(
                "INSERT INTO streams (subject, entity, version) VALUES ($1, $2, $3)
                 ON CONFLICT (subject) DO NOTHING",
                subject,
                E::NAME,
                count,
            )
            .execute(&mut *tx)
            .await
        } else {
            sqlx::query!(
                "UPDATE streams SET version = $3 WHERE subject = $1 AND version = $2",
                subject,
                expected,
                expected + count,
            )
            .execute(&mut *tx)
            .await
        }
        .map_err(StorageError::backend)?
        .rows_affected();
        if claimed == 0 {
            let actual =
                sqlx::query_scalar!("SELECT version FROM streams WHERE subject = $1", subject)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(StorageError::backend)?
                    .unwrap_or_default();
            return Err(StorageError::Conflict {
                expected: entity.version(),
                actual: Version::from(u64::try_from(actual).unwrap_or_default()),
            });
        }
        let last =
            sqlx::query_scalar!(r#"SELECT COALESCE(MAX(sequence), 0) AS "last!" FROM events"#)
                .fetch_one(&mut *tx)
                .await
                .map_err(StorageError::backend)?;
        for (offset, (id, event)) in (1..).zip(encoded) {
            sqlx::query!(
                "INSERT INTO events (sequence, id, type, schema_version, subject, stream_version,
                                     time, actor, correlation_id, causation_id, data)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                last + offset,
                id.to_string(),
                event.kind,
                i16::try_from(event.schema_version).map_err(StorageError::backend)?,
                subject,
                expected + offset,
                time as _,
                actor,
                correlation,
                causation,
                event.data,
            )
            .execute(&mut *tx)
            .await
            .map_err(StorageError::backend)?;
        }
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(PgDatabase::CHANNEL)
            .bind((last + count).to_string())
            .execute(&mut *tx)
            .await
            .map_err(StorageError::backend)?;
        tx.commit().await.map_err(StorageError::backend)?;
        entity.advance(events.len());
        Ok(())
    }

    async fn ids(&self) -> Result<Vec<Id<E>>, StorageError> {
        let subjects = sqlx::query_scalar!(
            "SELECT subject FROM streams WHERE entity = $1 ORDER BY subject",
            E::NAME,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::backend)?;
        subjects
            .iter()
            .map(|subject| subject.parse().map_err(StorageError::backend))
            .collect()
    }
}
