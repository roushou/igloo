use std::pin::Pin;

use async_trait::async_trait;
use igloo_core::Digest;
use tokio::io::AsyncRead;

use super::StorageError;

/// A stream of blob bytes.
pub type BlobReader = Pin<Box<dyn AsyncRead + Send>>;

/// Content-addressed bytes: snapshot layers and manifests.
///
/// Invariant: the bytes stored under a digest always hash to that digest.
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Stores `data` under `digest`, rejecting it unless its blake3 hash is `digest`. Storing
    /// the same blob again is a no-op.
    async fn put(&self, digest: Digest, data: BlobReader) -> Result<(), BlobError>;

    /// The bytes stored under `digest`, if any.
    async fn get(&self, digest: Digest) -> Result<Option<BlobReader>, BlobError>;

    /// Whether a blob is stored under `digest`.
    async fn contains(&self, digest: Digest) -> Result<bool, BlobError>;
}

/// Why a blob operation failed.
#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    /// The bytes hash to another digest.
    #[error("content hashes to {actual}, not {expected}")]
    DigestMismatch {
        /// The digest the bytes were stored under.
        expected: Digest,
        /// The digest of the bytes.
        actual: Digest,
    },
    /// The storage failed.
    #[error(transparent)]
    Storage(#[from] StorageError),
}
