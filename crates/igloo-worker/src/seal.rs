use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use igloo_core::sandbox::SandboxId;
use reqwest::Url;

use crate::blobs::{BlobSource, BlobSourceError};
use crate::layers::LayerCache;
use crate::rootfs::{Packed, Rootfs};
use crate::store::LocalStore;

/// Seals sandboxes: packs a sandbox's file system as a layer and uploads it to a seal's
/// presigned URL, marking a whole-root layer with `layer=full`. The layer is also cached here,
/// so forks placed on this worker start without downloading it.
#[derive(Clone)]
pub(crate) struct Sealer {
    source: Arc<dyn BlobSource>,
    layers: LayerCache,
    rootfs: Rootfs,
    store: LocalStore,
}

/// Why a seal failed on the worker.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SealError {
    #[error("the sandbox is not on this worker")]
    UnknownSandbox,
    #[error("packing the file system failed: {0}")]
    Pack(#[from] io::Error),
    #[error("uploading the layer failed: {0}")]
    Upload(#[from] BlobSourceError),
    #[error("the upload URL is invalid")]
    Url,
}

impl Sealer {
    pub(crate) const fn new(
        source: Arc<dyn BlobSource>,
        layers: LayerCache,
        rootfs: Rootfs,
        store: LocalStore,
    ) -> Self {
        Self {
            source,
            layers,
            rootfs,
            store,
        }
    }

    /// Packs sandbox `sandbox` and uploads the layer for seal `seal` to `url`.
    pub(crate) async fn seal(
        &self,
        seal: &str,
        sandbox: SandboxId,
        url: &str,
    ) -> Result<(), SealError> {
        let mut url: Url = url.parse().map_err(|_| SealError::Url)?;
        let sandbox_dir = self.store.sandbox_dir(sandbox);
        if !tokio::fs::try_exists(&sandbox_dir).await? {
            return Err(SealError::UnknownSandbox);
        }
        let archive = self.archive(seal).await?;
        let rootfs = self.rootfs;
        let target = archive.clone();
        let packed = tokio::task::spawn_blocking(move || {
            let file = std::fs::File::create(&target)?;
            rootfs.pack(&sandbox_dir, io::BufWriter::new(file))
        })
        .await
        .map_err(io::Error::other)
        .and_then(|packed| packed);
        let uploaded = match packed {
            Ok(packed) => {
                let digest = LayerCache::digest(&archive).await?;
                url.query_pairs_mut()
                    .append_pair("digest", &digest.to_string());
                if packed == Packed::Full {
                    url.query_pairs_mut().append_pair("layer", "full");
                }
                let file = tokio::fs::File::open(&archive).await?;
                let uploaded = self
                    .source
                    .upload(&url, file)
                    .await
                    .map_err(SealError::from);
                if uploaded.is_ok()
                    && let Err(error) = self.layers.adopt(&archive, digest).await
                {
                    tracing::warn!(%error, "caching the sealed layer failed");
                }
                uploaded
            }
            Err(error) => Err(error.into()),
        };
        if let Err(error) = tokio::fs::remove_file(&archive).await
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "removing a sealed archive failed");
        }
        uploaded
    }

    async fn archive(&self, seal: &str) -> io::Result<PathBuf> {
        let dir = self.store.root().join("seals");
        tokio::fs::create_dir_all(&dir).await?;
        let name = seal
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .chain(".tar".chars())
            .collect::<String>();
        Ok(dir.join(name))
    }
}
