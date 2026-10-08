use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use igloo_core::Digest;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::ports::{BlobError, BlobReader, BlobStore, StorageError};

/// Blobs as files under a root directory: `<root>/blake3/<2 hex>/<62 hex>`. Writes go to a
/// temporary file renamed into place once verified, so readers never see partial blobs.
#[derive(Debug)]
pub struct FsBlobStore {
    root: PathBuf,
    uploads: AtomicU64,
}

impl FsBlobStore {
    /// A store under `root`, created if missing.
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let root = root.into();
        tokio::fs::create_dir_all(root.join("tmp"))
            .await
            .map_err(StorageError::backend)?;
        Ok(Self {
            root,
            uploads: AtomicU64::new(0),
        })
    }

    fn path(&self, digest: &Digest) -> PathBuf {
        let text = digest.to_string();
        let hex = text.strip_prefix("blake3:").unwrap_or(&text);
        let (shard, rest) = hex.split_at(2);
        self.root.join("blake3").join(shard).join(rest)
    }

    async fn write_verified(
        &self,
        digest: Digest,
        mut data: BlobReader,
        temporary: &Path,
    ) -> Result<(), BlobError> {
        let mut file = tokio::fs::File::create(temporary)
            .await
            .map_err(StorageError::backend)?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = data
                .read(&mut buffer)
                .await
                .map_err(StorageError::backend)?;
            if read == 0 {
                break;
            }
            let chunk = buffer.get(..read).unwrap_or_default();
            hasher.update(chunk);
            file.write_all(chunk).await.map_err(StorageError::backend)?;
        }
        file.sync_all().await.map_err(StorageError::backend)?;
        let actual = Digest::from_blake3(*hasher.finalize().as_bytes());
        if actual != digest {
            return Err(BlobError::DigestMismatch {
                expected: digest,
                actual,
            });
        }
        let path = self.path(&digest);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(StorageError::backend)?;
        }
        tokio::fs::rename(temporary, &path)
            .await
            .map_err(StorageError::backend)?;
        Ok(())
    }
}

#[async_trait]
impl BlobStore for FsBlobStore {
    async fn put(&self, digest: Digest, data: BlobReader) -> Result<(), BlobError> {
        if self.contains(digest).await? {
            return Ok(());
        }
        let upload = self.uploads.fetch_add(1, Ordering::Relaxed);
        let temporary = self
            .root
            .join("tmp")
            .join(format!("{}-{upload}", std::process::id()));
        let result = self.write_verified(digest, data, &temporary).await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&temporary).await;
        }
        result
    }

    async fn get(&self, digest: Digest) -> Result<Option<BlobReader>, BlobError> {
        match tokio::fs::File::open(self.path(&digest)).await {
            Ok(file) => Ok(Some(Box::pin(file))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(StorageError::backend(error).into()),
        }
    }

    async fn contains(&self, digest: Digest) -> Result<bool, BlobError> {
        tokio::fs::try_exists(self.path(&digest))
            .await
            .map_err(|error| StorageError::backend(error).into())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::FsBlobStore;
    use crate::ports::conformance::BlobStoreConformance;

    #[tokio::test]
    async fn passes_the_blob_store_conformance_suite() {
        let root = tempfile::tempdir().expect("temporary directory");
        let blobs = FsBlobStore::new(root.path()).await.expect("store");
        BlobStoreConformance {
            blobs: Arc::new(blobs),
        }
        .run_all()
        .await;
    }
}
