use std::io::Cursor;
use std::sync::Arc;

use async_trait::async_trait;
use igloo_core::process::EnvVars;
use igloo_core::snapshot::{SnapshotId, SnapshotLayer, SnapshotManifest};
use igloo_core::{Digest, ErrorCode, ValidationErrors};
use tokio::io::AsyncReadExt;

use crate::app::{
    AppError, Command, CommandHandler, Extension, InstallError, PlatformBuilder, RequestContext,
};
use crate::ports::{
    BlobError, BlobReader, BlobStore, ImageReference, ImageRegistry, Platform, RegistryError,
    StorageError,
};

/// Stores bytes under their digest. The bytes may stream from an upload, so the command may take
/// up to an hour.
pub struct StoreBlob {
    /// The blake3 digest the bytes must hash to.
    pub digest: Digest,
    /// The bytes.
    pub data: BlobReader,
}

/// Registers a snapshot whose layers are stored blobs: `base`'s layers, then `layers`, with
/// `base`'s environment overlaid by `env`.
pub struct RegisterSnapshot {
    /// The snapshot whose layers come first.
    pub base: Option<SnapshotId>,
    /// The layers added over `base`, lowest first.
    pub layers: Vec<SnapshotLayer>,
    /// Environment set over `base`'s.
    pub env: EnvVars,
}

impl Command for StoreBlob {
    type Output = ();
    const NAME: &'static str = "blob.store";
    const TIMEOUT: Option<std::time::Duration> = Some(std::time::Duration::from_secs(3600));
}

impl Command for RegisterSnapshot {
    type Output = SnapshotId;
    const NAME: &'static str = "snapshot.register";
}

/// Why a snapshot cannot be read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotError {
    /// The stored manifest does not parse.
    #[error("stored snapshot manifest is malformed")]
    Malformed,
}

impl ErrorCode for SnapshotError {
    fn code(&self) -> &'static str {
        match self {
            Self::Malformed => "snapshot.malformed",
        }
    }
}

/// Snapshots stored as blobs: a snapshot's manifest is the blob under its id, so a snapshot
/// exists exactly when that blob does.
#[derive(Clone)]
pub struct Snapshots {
    blobs: Arc<dyn BlobStore>,
}

impl Snapshots {
    /// Snapshots over `blobs`.
    #[must_use]
    pub fn new(blobs: Arc<dyn BlobStore>) -> Self {
        Self { blobs }
    }

    /// Registers `layers` over `base`'s, with `base`'s environment overlaid by `env`; returns
    /// the new snapshot's id.
    pub async fn extend(
        &self,
        base: Option<SnapshotId>,
        layers: Vec<SnapshotLayer>,
        env: &EnvVars,
    ) -> Result<SnapshotId, AppError> {
        let manifest = match base {
            None => SnapshotManifest::try_from(layers),
            Some(base) => self
                .manifest(base)
                .await?
                .ok_or_else(|| {
                    AppError::Validation(ValidationErrors::single(
                        "base",
                        format!("no snapshot {base}"),
                    ))
                })?
                .extended(layers),
        }
        .map_err(|error| {
            AppError::Validation(ValidationErrors::single("layers", error.to_string()))
        })?;
        let env = manifest.env().overlaid(env).map_err(AppError::Validation)?;
        self.register(&manifest.with_env(env)).await
    }

    /// Registers `layers` in place of `base`'s, keeping its environment; returns the new
    /// snapshot's id.
    pub async fn replace(
        &self,
        base: SnapshotId,
        layers: Vec<SnapshotLayer>,
    ) -> Result<SnapshotId, AppError> {
        let manifest = self
            .manifest(base)
            .await?
            .ok_or_else(|| {
                AppError::Validation(ValidationErrors::single(
                    "base",
                    format!("no snapshot {base}"),
                ))
            })?
            .replaced(layers)
            .map_err(|error| {
                AppError::Validation(ValidationErrors::single("layers", error.to_string()))
            })?;
        self.register(&manifest).await
    }

    /// Stores `manifest` once every layer is stored; returns its id.
    pub async fn register(&self, manifest: &SnapshotManifest) -> Result<SnapshotId, AppError> {
        let mut missing = ValidationErrors::default();
        for (index, layer) in manifest.layers().iter().enumerate() {
            if !self.blobs.contains(layer.digest()).await? {
                missing.add(
                    format!("layers.{index}"),
                    format!("no blob {}", layer.digest()),
                );
            }
        }
        missing.into_result(())?;
        let bytes = manifest.canonical_bytes();
        let digest = Digest::from_blake3(*blake3::hash(&bytes).as_bytes());
        self.blobs
            .put(digest, Box::pin(Cursor::new(bytes)))
            .await
            .map_err(AppError::from)?;
        Ok(SnapshotId::from(digest))
    }

    /// The manifest of snapshot `id`, if registered.
    pub async fn manifest(&self, id: SnapshotId) -> Result<Option<SnapshotManifest>, AppError> {
        let Some(mut reader) = self
            .blobs
            .get(*id.as_digest())
            .await
            .map_err(AppError::from)?
        else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| AppError::from(StorageError::backend(error)))?;
        SnapshotManifest::try_from(bytes.as_slice())
            .map(Some)
            .map_err(|_| AppError::domain(&SnapshotError::Malformed))
    }

    /// Whether snapshot `id` is registered.
    pub async fn exists(&self, id: SnapshotId) -> Result<bool, AppError> {
        self.blobs
            .contains(*id.as_digest())
            .await
            .map_err(AppError::from)
    }
}

/// Imports container images into the blob store.
#[derive(Clone)]
pub struct ImageImporter {
    registry: Arc<dyn ImageRegistry>,
    blobs: Arc<dyn BlobStore>,
}

impl ImageImporter {
    /// An importer pulling from `registry` into `blobs`.
    #[must_use]
    pub fn new(registry: Arc<dyn ImageRegistry>, blobs: Arc<dyn BlobStore>) -> Self {
        Self { registry, blobs }
    }

    /// Pulls `image` for `platform` and stores the layers not yet stored; returns them lowest
    /// first with the image's environment, ready for [`RegisterSnapshot`]. Environment entries
    /// whose names sandboxes cannot carry are dropped.
    pub async fn import(
        &self,
        image: &ImageReference,
        platform: &Platform,
    ) -> Result<(Vec<SnapshotLayer>, EnvVars), AppError> {
        let pulled = self
            .registry
            .pull(image, platform)
            .await
            .map_err(|error| match error {
                RegistryError::NotFound => AppError::not_found("image", image),
                RegistryError::NoMatchingPlatform(_) => {
                    AppError::Validation(ValidationErrors::single("platform", error.to_string()))
                }
                RegistryError::UnsupportedMediaType(_) => {
                    AppError::Validation(ValidationErrors::single("image", error.to_string()))
                }
                RegistryError::DigestMismatch(_) | RegistryError::Transport(_) => {
                    AppError::infrastructure(error)
                }
            })?;
        let env = pulled
            .env
            .into_iter()
            .filter(|(key, value)| EnvVars::from_pairs([(key.as_str(), value.as_str())]).is_ok());
        let env = EnvVars::from_pairs(env).unwrap_or_default();
        let mut layers = Vec::with_capacity(pulled.layers.len());
        for layer in pulled.layers {
            if !self.blobs.contains(layer.digest).await? {
                let file = tokio::fs::File::open(&layer.file)
                    .await
                    .map_err(|error| AppError::from(StorageError::backend(error)))?;
                self.blobs.put(layer.digest, Box::pin(file)).await?;
            }
            layers.push(SnapshotLayer::new(layer.digest, layer.media_type));
        }
        Ok((layers, env))
    }
}

impl From<BlobError> for AppError {
    fn from(error: BlobError) -> Self {
        match error {
            BlobError::DigestMismatch { .. } => {
                Self::Validation(ValidationErrors::single("digest", error.to_string()))
            }
            BlobError::Storage(error) => error.into(),
        }
    }
}

struct StoreBlobHandler {
    blobs: Arc<dyn BlobStore>,
}

#[async_trait]
impl CommandHandler<StoreBlob> for StoreBlobHandler {
    async fn handle(&self, command: StoreBlob, _: &RequestContext) -> Result<(), AppError> {
        Ok(self.blobs.put(command.digest, command.data).await?)
    }
}

struct RegisterSnapshotHandler {
    snapshots: Snapshots,
}

#[async_trait]
impl CommandHandler<RegisterSnapshot> for RegisterSnapshotHandler {
    async fn handle(
        &self,
        command: RegisterSnapshot,
        _: &RequestContext,
    ) -> Result<SnapshotId, AppError> {
        self.snapshots
            .extend(command.base, command.layers, &command.env)
            .await
    }
}

/// Blobs and snapshots.
pub struct SnapshotModule;

impl Extension for SnapshotModule {
    fn name(&self) -> &'static str {
        "snapshot"
    }

    fn install(self: Box<Self>, platform: &mut PlatformBuilder) -> Result<(), InstallError> {
        let blobs = Arc::clone(&platform.ports().blobs);
        platform.command(StoreBlobHandler {
            blobs: Arc::clone(&blobs),
        })?;
        platform.command(RegisterSnapshotHandler {
            snapshots: Snapshots::new(blobs),
        })
    }
}
