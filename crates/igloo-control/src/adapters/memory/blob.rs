use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use igloo_core::{Digest, Timestamp};
use tokio::io::AsyncReadExt;

use crate::ports::{BlobError, BlobInfo, BlobReader, BlobStore, Clock, StorageError};

/// A blob's bytes and when they were stored.
type Stored = (Arc<[u8]>, Timestamp);

/// Blobs in a map, stamped by a [`Clock`].
pub struct MemoryBlobStore {
    clock: Arc<dyn Clock>,
    blobs: Mutex<HashMap<Digest, Stored>>,
}

impl MemoryBlobStore {
    /// An empty store stamping blobs with `clock`.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            blobs: Mutex::default(),
        }
    }
}

#[async_trait]
impl BlobStore for MemoryBlobStore {
    async fn put(&self, digest: Digest, mut data: BlobReader) -> Result<(), BlobError> {
        let mut bytes = Vec::new();
        data.read_to_end(&mut bytes)
            .await
            .map_err(StorageError::backend)?;
        let actual = Digest::from_blake3(*blake3::hash(&bytes).as_bytes());
        if actual != digest {
            return Err(BlobError::DigestMismatch {
                expected: digest,
                actual,
            });
        }
        self.blobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(digest, (bytes.into(), self.clock.now()));
        Ok(())
    }

    async fn get(&self, digest: Digest) -> Result<Option<BlobReader>, BlobError> {
        let blobs = self
            .blobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(blobs
            .get(&digest)
            .map(|(bytes, _)| Box::pin(Cursor::new(Arc::clone(bytes))) as BlobReader))
    }

    async fn contains(&self, digest: Digest) -> Result<bool, BlobError> {
        let blobs = self
            .blobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(blobs.contains_key(&digest))
    }

    async fn list(&self) -> Result<Vec<BlobInfo>, BlobError> {
        let blobs = self
            .blobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(blobs
            .iter()
            .map(|(digest, (bytes, stored_at))| BlobInfo {
                digest: *digest,
                size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                stored_at: *stored_at,
            })
            .collect())
    }

    async fn delete(&self, digest: Digest) -> Result<(), BlobError> {
        self.blobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&digest);
        Ok(())
    }
}
