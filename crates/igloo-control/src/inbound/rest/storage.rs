use axum::Json;
use axum::extract::State;
use igloo_api::storage::{StorageResource, StorageSnapshot, StorageSweep};

use super::auth::Caller;
use super::{ApiError, ApiState};
use crate::platform::{SnapshotSize, StorageUsage, Sweep};

/// Reports what the blob store holds and what its latest sweep reclaimed.
#[utoipa::path(
    get,
    operation_id = "getStorage",
    path = "/v1/storage",
    tag = "storage",
    responses((status = 200, body = StorageResource))
)]
pub(super) async fn get(
    State(state): State<ApiState>,
    _: Caller,
) -> Result<Json<StorageResource>, ApiError> {
    Ok(Json(
        state
            .collector
            .usage()
            .await
            .map_err(ApiError::from)?
            .into(),
    ))
}

impl From<StorageUsage> for StorageResource {
    fn from(usage: StorageUsage) -> Self {
        Self::new(
            usage.blobs,
            usage.bytes,
            usage.last_sweep.map(StorageSweep::from),
            usage
                .snapshots
                .into_iter()
                .map(StorageSnapshot::from)
                .collect(),
        )
    }
}

impl From<Sweep> for StorageSweep {
    fn from(sweep: Sweep) -> Self {
        Self::new(
            sweep.at,
            sweep.kept_blobs,
            sweep.kept_bytes,
            sweep.reclaimed_blobs,
            sweep.reclaimed_bytes,
        )
    }
}

impl From<SnapshotSize> for StorageSnapshot {
    fn from(size: SnapshotSize) -> Self {
        Self::new(
            size.repo.to_string(),
            size.key.to_string(),
            size.snapshot.to_string(),
            size.size_bytes,
        )
    }
}
