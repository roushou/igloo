use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use igloo_core::{Digest, Timestamp};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::ports::{BlobError, BlobInfo, BlobReader, BlobStore, Clock, StorageError};

/// Blobs as files under a root directory: `<root>/blake3/<2 hex>/<62 hex>`. Writes go to a
/// temporary file renamed into place once verified, so readers never see partial blobs.
pub struct FsBlobStore {
    root: PathBuf,
    clock: Arc<dyn Clock>,
    uploads: AtomicU64,
}

impl FsBlobStore {
    /// A store under `root`, created if missing. Storing a blob that exists again stamps it with
    /// `clock`.
    pub async fn new(
        root: impl Into<PathBuf>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, StorageError> {
        let root = root.into();
        tokio::fs::create_dir_all(root.join("tmp"))
            .await
            .map_err(StorageError::backend)?;
        Ok(Self {
            root,
            clock,
            uploads: AtomicU64::new(0),
        })
    }

    fn path(&self, digest: &Digest) -> PathBuf {
        let text = digest.to_string();
        let hex = text.strip_prefix("blake3:").unwrap_or(&text);
        let (shard, rest) = hex.split_at(2);
        self.root.join("blake3").join(shard).join(rest)
    }

    /// Sets the blob's modification time to the present.
    async fn touch(&self, digest: Digest) -> Result<(), BlobError> {
        let file = match tokio::fs::OpenOptions::new()
            .write(true)
            .open(self.path(&digest))
            .await
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(StorageError::backend(error).into()),
        };
        file.into_std()
            .await
            .set_modified(std::time::SystemTime::from(jiff::Timestamp::from(
                self.clock.now(),
            )))
            .map_err(StorageError::backend)?;
        Ok(())
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
            return self.touch(digest).await;
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

    async fn list(&self) -> Result<Vec<BlobInfo>, BlobError> {
        let mut blobs = Vec::new();
        let mut shards = tokio::fs::read_dir(self.root.join("blake3"))
            .await
            .map_err(StorageError::backend)?;
        while let Some(shard) = shards.next_entry().await.map_err(StorageError::backend)? {
            let prefix = shard.file_name().to_string_lossy().into_owned();
            let mut files = tokio::fs::read_dir(shard.path())
                .await
                .map_err(StorageError::backend)?;
            while let Some(file) = files.next_entry().await.map_err(StorageError::backend)? {
                let name = file.file_name().to_string_lossy().into_owned();
                let Ok(digest) = format!("blake3:{prefix}{name}").parse::<Digest>() else {
                    continue;
                };
                let meta = match file.metadata().await {
                    Ok(meta) => meta,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(StorageError::backend(error).into()),
                };
                let stored_at = meta.modified().map_err(StorageError::backend)?;
                blobs.push(BlobInfo {
                    digest,
                    size: meta.len(),
                    stored_at: Timestamp::new(
                        jiff::Timestamp::try_from(stored_at).map_err(StorageError::backend)?,
                    ),
                });
            }
        }
        Ok(blobs)
    }

    async fn delete(&self, digest: Digest) -> Result<(), BlobError> {
        match tokio::fs::remove_file(self.path(&digest)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(StorageError::backend(error).into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::FsBlobStore;
    use crate::adapters::SystemClock;
    use crate::ports::conformance::BlobStoreConformance;

    #[tokio::test]
    async fn passes_the_blob_store_conformance_suite() {
        let root = tempfile::tempdir().expect("temporary directory");
        let blobs = FsBlobStore::new(root.path(), Arc::new(SystemClock))
            .await
            .expect("store");
        BlobStoreConformance {
            blobs: Arc::new(blobs),
        }
        .run_all()
        .await;
    }
}
