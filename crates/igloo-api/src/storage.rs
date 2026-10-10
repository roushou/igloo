//! The server's blob store.

use igloo_core::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What the blob store holds and what its latest sweep reclaimed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct StorageResource {
    /// Blobs stored: layers and snapshot manifests.
    pub blobs: u64,
    /// Their total size in bytes.
    pub bytes: u64,
    /// The latest sweep since the server started; absent until one has run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sweep: Option<StorageSweep>,
    /// The repositories' recorded warm and agent snapshots, ordered by repository then key.
    pub snapshots: Vec<StorageSnapshot>,
}

/// What one sweep kept and reclaimed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct StorageSweep {
    /// When the sweep finished.
    #[schema(value_type = String, format = DateTime)]
    pub at: Timestamp,
    /// Blobs left in the store.
    pub kept_blobs: u64,
    /// Their size in bytes.
    pub kept_bytes: u64,
    /// Blobs the sweep deleted.
    pub reclaimed_blobs: u64,
    /// Their size in bytes.
    pub reclaimed_bytes: u64,
}

/// A recorded warm or agent snapshot and its size.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct StorageSnapshot {
    /// The repository that recorded it (`repo_...`).
    pub repo: String,
    /// The key it is recorded under.
    pub key: String,
    /// The snapshot's id.
    pub snapshot: String,
    /// The sum of its layers' sizes in bytes, from file metadata; layers shared with other
    /// snapshots count in each.
    pub size_bytes: u64,
}

impl StorageResource {
    /// A store of `blobs` totalling `bytes`.
    #[must_use]
    pub const fn new(
        blobs: u64,
        bytes: u64,
        last_sweep: Option<StorageSweep>,
        snapshots: Vec<StorageSnapshot>,
    ) -> Self {
        Self {
            blobs,
            bytes,
            last_sweep,
            snapshots,
        }
    }
}

impl StorageSweep {
    /// A sweep that finished at `at`.
    #[must_use]
    pub const fn new(
        at: Timestamp,
        kept_blobs: u64,
        kept_bytes: u64,
        reclaimed_blobs: u64,
        reclaimed_bytes: u64,
    ) -> Self {
        Self {
            at,
            kept_blobs,
            kept_bytes,
            reclaimed_blobs,
            reclaimed_bytes,
        }
    }
}

impl StorageSnapshot {
    /// A snapshot of `size_bytes`.
    #[must_use]
    pub const fn new(repo: String, key: String, snapshot: String, size_bytes: u64) -> Self {
        Self {
            repo,
            key,
            snapshot,
            size_bytes,
        }
    }
}
