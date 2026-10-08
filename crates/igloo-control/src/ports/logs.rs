use async_trait::async_trait;
use igloo_core::job::JobId;
use igloo_core::process::OutputStream;

use super::StorageError;

/// One stored chunk of a job's output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    /// The chunk's position among the job's entries; increases in append order.
    pub sequence: u64,
    /// The stream it came from.
    pub stream: OutputStream,
    /// The bytes.
    pub data: Vec<u8>,
}

/// Job output, as appended by the gateway and read by clients.
#[async_trait]
pub trait LogStore: Send + Sync {
    /// Stores `data` found at `offset` of `stream`. A chunk already stored at that offset is
    /// ignored, so resent output is not duplicated.
    async fn append(
        &self,
        job: JobId,
        stream: OutputStream,
        offset: u64,
        data: &[u8],
    ) -> Result<(), StorageError>;

    /// Up to `limit` entries of `job` with a sequence greater than `after`, in order.
    async fn read(
        &self,
        job: JobId,
        after: u64,
        limit: usize,
    ) -> Result<Vec<LogEntry>, StorageError>;
}
