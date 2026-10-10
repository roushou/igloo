//! Snapshots: content-addressed filesystem images sandboxes start from.

use igloo_core::snapshot::{
    MediaType as CoreMediaType, SnapshotId, SnapshotLayer, SnapshotManifest,
};
use igloo_core::{Digest, Timestamp, ValidationErrors};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// How a layer blob is encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub enum MediaType {
    /// A tar archive.
    #[default]
    #[serde(rename = "tar")]
    Tar,
    /// A gzip-compressed tar archive.
    #[serde(rename = "tar+gzip")]
    TarGzip,
}

/// One layer of a snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct Layer {
    /// The uploaded blob's digest.
    pub digest: String,
    /// How the blob is encoded; `tar` when omitted.
    #[serde(default)]
    pub media_type: MediaType,
}

/// Registers a snapshot from uploaded blobs, optionally on top of an existing snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct CreateSnapshotRequest {
    /// A snapshot whose layers go underneath, such as an imported image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Layers, lowest first.
    pub layers: Vec<Layer>,
}

/// Imports a public OCI image as a snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct ImportImageRequest {
    /// An image reference, such as `docker.io/library/rust:1.99` or `rust:1.99`.
    pub image: String,
    /// `os/architecture`, such as `linux/arm64`; the server's architecture when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}

/// A snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct SnapshotResource {
    /// Its id: the digest of its manifest.
    pub id: String,
    /// Layers, lowest first.
    pub layers: Vec<Layer>,
}

impl Layer {
    /// A layer stored under `digest`, encoded as `media_type`.
    #[must_use]
    pub fn new(digest: impl Into<String>, media_type: MediaType) -> Self {
        Self {
            digest: digest.into(),
            media_type,
        }
    }
}

impl CreateSnapshotRequest {
    /// A request registering `layers`, lowest first.
    #[must_use]
    pub const fn new(layers: Vec<Layer>) -> Self {
        Self { base: None, layers }
    }

    /// Puts `base`'s layers underneath.
    #[must_use]
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = Some(base.into());
        self
    }

    /// The layers as domain values, reporting every invalid one. The base is resolved by the
    /// server.
    pub fn layers(&self) -> Result<Vec<SnapshotLayer>, ValidationErrors> {
        let mut errors = ValidationErrors::default();
        let mut layers = Vec::new();
        for (index, layer) in self.layers.iter().enumerate() {
            match layer.digest.parse::<Digest>() {
                Ok(digest) => layers.push(SnapshotLayer::new(digest, layer.media_type.into())),
                Err(error) => errors.add(format!("layers.{index}.digest"), error.to_string()),
            }
        }
        errors.into_result(layers)
    }
}

impl ImportImageRequest {
    /// A request importing `image` for the server's platform.
    #[must_use]
    pub fn new(image: impl Into<String>) -> Self {
        Self {
            image: image.into(),
            platform: None,
        }
    }

    /// Chooses the platform, `os/architecture`.
    #[must_use]
    pub fn with_platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = Some(platform.into());
        self
    }
}

impl From<MediaType> for CoreMediaType {
    fn from(media_type: MediaType) -> Self {
        match media_type {
            MediaType::Tar => Self::Tar,
            MediaType::TarGzip => Self::TarGzip,
        }
    }
}

impl From<CoreMediaType> for MediaType {
    fn from(media_type: CoreMediaType) -> Self {
        match media_type {
            CoreMediaType::Tar => Self::Tar,
            CoreMediaType::TarGzip => Self::TarGzip,
        }
    }
}

impl From<(SnapshotId, &SnapshotManifest)> for SnapshotResource {
    fn from((id, manifest): (SnapshotId, &SnapshotManifest)) -> Self {
        Self {
            id: id.to_string(),
            layers: manifest
                .layers()
                .iter()
                .map(|layer| Layer::new(layer.digest().to_string(), layer.media_type().into()))
                .collect(),
        }
    }
}

/// A snapshot a repository recorded under a key: a warm snapshot, or an agent snapshot over one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct WarmSnapshotResource {
    /// What it was built from: a digest of its base, build command and lockfiles.
    pub key: String,
    /// The snapshot's id.
    pub snapshot: String,
    /// The commit it was built at.
    pub commit: String,
    /// The bytes of its layer blobs; absent until the blob store reports sizes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// When it was recorded, from the event log; absent when the log no longer holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub built_at: Option<Timestamp>,
    /// When a run's checkout or a task's sandbox was last made over it, at hour precision; absent
    /// until a use is recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_used_at: Option<Timestamp>,
}

impl WarmSnapshotResource {
    /// The snapshot `snapshot` recorded under `key`, built at `commit`.
    #[must_use]
    pub fn new(
        key: impl Into<String>,
        snapshot: impl Into<String>,
        commit: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            snapshot: snapshot.into(),
            commit: commit.into(),
            size_bytes: None,
            built_at: None,
            last_used_at: None,
        }
    }

    /// Sets the size of its layers.
    #[must_use]
    pub const fn with_size_bytes(mut self, bytes: u64) -> Self {
        self.size_bytes = Some(bytes);
        self
    }

    /// Sets when it was recorded.
    #[must_use]
    pub const fn with_built_at(mut self, at: Timestamp) -> Self {
        self.built_at = Some(at);
        self
    }

    /// Sets when it was last used.
    #[must_use]
    pub const fn with_last_used_at(mut self, at: Timestamp) -> Self {
        self.last_used_at = Some(at);
        self
    }
}
