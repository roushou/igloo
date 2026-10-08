use async_trait::async_trait;
use igloo_core::job::JobId;
use igloo_core::process::OutputStream;
use sqlx::PgPool;

use super::PgDatabase;
use crate::ports::{LogEntry, LogStore, StorageError};

/// Job output in the `job_logs` table.
pub struct PgLogStore {
    pool: PgPool,
}

impl PgLogStore {
    /// Logs over `database`.
    #[must_use]
    pub fn new(database: &PgDatabase) -> Self {
        Self {
            pool: database.pool().clone(),
        }
    }

    const fn column(stream: OutputStream) -> &'static str {
        match stream {
            OutputStream::Stdout => "stdout",
            OutputStream::Stderr => "stderr",
        }
    }
}

#[async_trait]
impl LogStore for PgLogStore {
    async fn append(
        &self,
        job: JobId,
        stream: OutputStream,
        offset: u64,
        data: &[u8],
    ) -> Result<(), StorageError> {
        let offset = i64::try_from(offset).map_err(StorageError::backend)?;
        sqlx::query!(
            "INSERT INTO job_logs (job_id, stream, byte_offset, data) VALUES ($1, $2, $3, $4)
             ON CONFLICT (job_id, stream, byte_offset) DO NOTHING",
            job.to_string(),
            Self::column(stream),
            offset,
            data,
        )
        .execute(&self.pool)
        .await
        .map_err(StorageError::backend)?;
        Ok(())
    }

    async fn read(
        &self,
        job: JobId,
        after: u64,
        limit: usize,
    ) -> Result<Vec<LogEntry>, StorageError> {
        let after = i64::try_from(after).map_err(StorageError::backend)?;
        let limit = i64::try_from(limit).map_err(StorageError::backend)?;
        let rows = sqlx::query!(
            "SELECT sequence, stream, data FROM job_logs
             WHERE job_id = $1 AND sequence > $2 ORDER BY sequence LIMIT $3",
            job.to_string(),
            after,
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::backend)?;
        rows.into_iter()
            .map(|row| {
                let stream = match row.stream.as_str() {
                    "stdout" => OutputStream::Stdout,
                    _ => OutputStream::Stderr,
                };
                Ok(LogEntry {
                    sequence: u64::try_from(row.sequence).map_err(StorageError::backend)?,
                    stream,
                    data: row.data,
                })
            })
            .collect()
    }
}
