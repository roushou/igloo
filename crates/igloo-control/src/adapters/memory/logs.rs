use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use igloo_core::job::JobId;
use igloo_core::process::OutputStream;

use crate::ports::{LogEntry, LogStore, StorageError};

/// Job output in a map.
#[derive(Debug, Default)]
pub struct MemoryLogStore {
    jobs: Mutex<HashMap<JobId, JobLog>>,
}

#[derive(Debug, Default)]
struct JobLog {
    entries: Vec<LogEntry>,
    offsets: HashSet<(OutputStream, u64)>,
}

#[async_trait]
impl LogStore for MemoryLogStore {
    async fn append(
        &self,
        job: JobId,
        stream: OutputStream,
        offset: u64,
        data: &[u8],
    ) -> Result<(), StorageError> {
        let mut jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        let log = jobs.entry(job).or_default();
        if log.offsets.insert((stream, offset)) {
            let sequence = log.entries.last().map_or(1, |last| last.sequence + 1);
            log.entries.push(LogEntry {
                sequence,
                stream,
                data: data.to_vec(),
            });
        }
        Ok(())
    }

    async fn read(
        &self,
        job: JobId,
        after: u64,
        limit: usize,
    ) -> Result<Vec<LogEntry>, StorageError> {
        let jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(jobs.get(&job).map_or_else(Vec::new, |log| {
            log.entries
                .iter()
                .filter(|entry| entry.sequence > after)
                .take(limit)
                .cloned()
                .collect()
        }))
    }
}
