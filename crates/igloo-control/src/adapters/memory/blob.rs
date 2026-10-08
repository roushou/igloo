use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use igloo_core::Digest;
use tokio::io::AsyncReadExt;

use crate::ports::{BlobError, BlobReader, BlobStore, StorageError};

/// Blobs in a map.
#[derive(Debug, Default)]
pub struct MemoryBlobStore {
    blobs: Mutex<HashMap<Digest, Arc<[u8]>>>,
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
            .insert(digest, bytes.into());
        Ok(())
    }

    async fn get(&self, digest: Digest) -> Result<Option<BlobReader>, BlobError> {
        let blobs = self
            .blobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(blobs
            .get(&digest)
            .map(|bytes| Box::pin(Cursor::new(Arc::clone(bytes))) as BlobReader))
    }

    async fn contains(&self, digest: Digest) -> Result<bool, BlobError> {
        let blobs = self
            .blobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(blobs.contains_key(&digest))
    }
}
