use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use igloo_core::Timestamp;
use sqlx::PgPool;
use sqlx::postgres::PgListener;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::PgDatabase;
use crate::ports::{EventEnvelope, EventLog, Sequence, StorageError};

/// The `events` table as the global log. Its head follows commits through LISTEN/NOTIFY, with
/// periodic polling in case a notification is lost.
pub struct PgEventLog {
    pool: PgPool,
    head: watch::Sender<Sequence>,
}

impl PgEventLog {
    /// How often the head is polled besides notifications.
    pub const POLL: Duration = Duration::from_secs(1);

    /// A log over `database`, its head initialised to the latest sequence.
    pub async fn new(database: &PgDatabase) -> Result<Arc<Self>, StorageError> {
        let log = Self {
            pool: database.pool().clone(),
            head: watch::Sender::new(Sequence::START),
        };
        log.refresh().await?;
        Ok(Arc::new(log))
    }

    /// Keeps the head current until `cancel` fires. Run it under the task supervisor.
    pub async fn follow(self: Arc<Self>, cancel: CancellationToken) -> Result<(), StorageError> {
        let mut listener = PgListener::connect_with(&self.pool)
            .await
            .map_err(StorageError::backend)?;
        listener
            .listen(PgDatabase::CHANNEL)
            .await
            .map_err(StorageError::backend)?;
        let mut poll = tokio::time::interval(Self::POLL);
        loop {
            tokio::select! {
                () = cancel.cancelled() => return Ok(()),
                notification = listener.recv() => {
                    if let Err(error) = notification {
                        tracing::warn!(%error, "event notification failed; polling continues");
                    }
                }
                _ = poll.tick() => {}
            }
            if let Err(error) = self.refresh().await {
                tracing::warn!(%error, "refreshing the event log head failed");
            }
        }
    }

    /// Publishes the latest committed sequence if it moved.
    async fn refresh(&self) -> Result<(), StorageError> {
        let latest =
            sqlx::query_scalar!(r#"SELECT COALESCE(MAX(sequence), 0) AS "latest!" FROM events"#)
                .fetch_one(&self.pool)
                .await
                .map_err(StorageError::backend)?;
        let latest = Sequence::from(u64::try_from(latest).map_err(StorageError::backend)?);
        self.head.send_if_modified(|head| {
            let moved = latest > *head;
            if moved {
                *head = latest;
            }
            moved
        });
        Ok(())
    }
}

#[async_trait]
impl EventLog for PgEventLog {
    async fn read(
        &self,
        after: Sequence,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StorageError> {
        let after = i64::try_from(after.get()).map_err(StorageError::backend)?;
        let limit = i64::try_from(limit).map_err(StorageError::backend)?;
        sqlx::query_as!(
            EventRow,
            r#"SELECT sequence, id, type AS kind, schema_version, subject, stream_version,
                      time AS "time: jiff_sqlx::Timestamp", actor, correlation_id, causation_id, data
               FROM events WHERE sequence > $1 ORDER BY sequence LIMIT $2"#,
            after,
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::backend)?
        .into_iter()
        .map(EventEnvelope::try_from)
        .collect()
    }

    fn head(&self) -> watch::Receiver<Sequence> {
        self.head.subscribe()
    }
}

/// One row of `events`.
struct EventRow {
    sequence: i64,
    id: String,
    kind: String,
    schema_version: i16,
    subject: String,
    stream_version: i64,
    time: jiff_sqlx::Timestamp,
    actor: serde_json::Value,
    correlation_id: String,
    causation_id: Option<String>,
    data: serde_json::Value,
}

impl TryFrom<EventRow> for EventEnvelope {
    type Error = StorageError;

    fn try_from(row: EventRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id.parse().map_err(StorageError::backend)?,
            kind: row.kind,
            schema_version: u16::try_from(row.schema_version).map_err(StorageError::backend)?,
            subject: row.subject,
            stream_version: u64::try_from(row.stream_version).map_err(StorageError::backend)?,
            sequence: Sequence::from(u64::try_from(row.sequence).map_err(StorageError::backend)?),
            time: Timestamp::from(row.time.to_jiff()),
            actor: serde_json::from_value(row.actor).map_err(StorageError::backend)?,
            correlation_id: row.correlation_id.parse().map_err(StorageError::backend)?,
            causation_id: row
                .causation_id
                .map(|id| id.parse())
                .transpose()
                .map_err(StorageError::backend)?,
            data: row.data,
        })
    }
}
