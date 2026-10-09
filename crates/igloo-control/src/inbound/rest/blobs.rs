use axum::Json;
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use igloo_api::problem::Problem;
use igloo_api::snapshot::{CreateSnapshotRequest, ImportImageRequest, SnapshotResource};
use igloo_core::process::EnvVars;
use igloo_core::snapshot::SnapshotId;
use igloo_core::{Digest, ValidationErrors, Validator};
use tokio_util::io::ReaderStream;

use super::auth::{BlobCaller, Caller};
use super::upload::Upload;
use super::{ApiError, ApiState};
use crate::app::AppError;
use crate::platform::{RegisterSnapshot, StoreBlob};
use crate::ports::{ImageReference, Platform};

/// Uploads bytes under their blake3 digest. Idempotent; rejected if the bytes do not match.
/// Authenticated by the bearer token or a presigned upload URL.
#[utoipa::path(
    put,
    operation_id = "putBlob",
    path = "/v1/blobs/{digest}",
    tag = "snapshots",
    params(
        ("digest" = String, Path, description = "`blake3:<64 hex>`"),
        ("expires" = Option<i64>, Query, description = "Presigned URL expiry, Unix seconds"),
        ("signature" = Option<String>, Query, description = "Presigned URL signature"),
    ),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses(
        (status = 204),
        (status = 403, body = Problem),
        (status = 413, body = Problem),
        (status = 422, body = Problem),
    )
)]
pub(super) async fn put(
    State(state): State<ApiState>,
    BlobCaller(context): BlobCaller,
    Path(digest): Path<String>,
    Upload(data): Upload,
) -> Result<StatusCode, ApiError> {
    let digest = digest.parse::<Digest>().map_err(|error| {
        AppError::Validation(ValidationErrors::single("digest", error.to_string()))
    })?;
    state
        .bus
        .dispatch(StoreBlob { digest, data }, context)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Downloads a blob. Authenticated by the bearer token or a presigned download URL.
#[utoipa::path(
    get,
    operation_id = "getBlob",
    path = "/v1/blobs/{digest}",
    tag = "snapshots",
    params(
        ("digest" = String, Path),
        ("expires" = Option<i64>, Query, description = "Presigned URL expiry, Unix seconds"),
        ("signature" = Option<String>, Query, description = "Presigned URL signature"),
    ),
    responses(
        (status = 200, content_type = "application/octet-stream", body = Vec<u8>),
        (status = 403, body = Problem),
        (status = 404, body = Problem),
    )
)]
pub(super) async fn get(
    State(state): State<ApiState>,
    _: BlobCaller,
    Path(digest): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let digest: Digest = digest
        .parse()
        .map_err(|_| ApiError::not_found("blob.not_found"))?;
    let reader = state
        .blobs
        .get(digest)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("blob.not_found"))?;
    Ok((
        [(header::CONTENT_TYPE, "application/octet-stream")],
        Body::from_stream(ReaderStream::new(reader)),
    ))
}

/// Registers a snapshot of uploaded layers, over a base snapshot's layers if one is named.
#[utoipa::path(
    post,
    operation_id = "createSnapshot",
    path = "/v1/snapshots",
    tag = "snapshots",
    request_body = CreateSnapshotRequest,
    responses((status = 201, body = SnapshotResource), (status = 422, body = Problem))
)]
pub(super) async fn create_snapshot(
    State(state): State<ApiState>,
    Caller(context): Caller,
    request: Result<Json<CreateSnapshotRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<SnapshotResource>), ApiError> {
    let Json(request) = request?;
    let (base, layers) = Validator::new()
        .field(
            "base",
            request
                .base
                .as_deref()
                .map(str::parse::<SnapshotId>)
                .transpose(),
        )
        .nested("", request.layers())
        .finish()
        .map_err(AppError::from)?;
    let id = state
        .bus
        .dispatch(
            RegisterSnapshot {
                base,
                layers,
                env: EnvVars::default(),
            },
            context,
        )
        .await?;
    state.registered(id).await
}

/// Pulls a public container image from its registry and registers its layers as a snapshot.
#[utoipa::path(
    post,
    operation_id = "importImage",
    path = "/v1/snapshots/import",
    tag = "snapshots",
    request_body = ImportImageRequest,
    responses(
        (status = 201, body = SnapshotResource),
        (status = 404, body = Problem),
        (status = 422, body = Problem),
    )
)]
pub(super) async fn import_image(
    State(state): State<ApiState>,
    Caller(context): Caller,
    request: Result<Json<ImportImageRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<SnapshotResource>), ApiError> {
    let Json(request) = request?;
    let (image, platform) = Validator::new()
        .field("image", request.image.parse::<ImageReference>())
        .field(
            "platform",
            request
                .platform
                .as_deref()
                .map_or_else(|| Ok(Platform::host()), str::parse::<Platform>),
        )
        .finish()
        .map_err(AppError::from)?;
    let (layers, env) = state.importer.import(&image, &platform).await?;
    let id = state
        .bus
        .dispatch(
            RegisterSnapshot {
                base: None,
                layers,
                env,
            },
            context,
        )
        .await?;
    state.registered(id).await
}

impl ApiState {
    /// The `201 Created` response for snapshot `id`, just registered.
    async fn registered(
        &self,
        id: SnapshotId,
    ) -> Result<(StatusCode, Json<SnapshotResource>), ApiError> {
        let manifest = self
            .snapshots
            .manifest(id)
            .await?
            .ok_or_else(|| ApiError::not_found("snapshot.not_found"))?;
        Ok((
            StatusCode::CREATED,
            Json(SnapshotResource::from((id, &manifest))),
        ))
    }
}

/// Gets a snapshot.
#[utoipa::path(
    get,
    operation_id = "getSnapshot",
    path = "/v1/snapshots/{id}",
    tag = "snapshots",
    params(("id" = String, Path)),
    responses((status = 200, body = SnapshotResource), (status = 404, body = Problem))
)]
pub(super) async fn get_snapshot(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
) -> Result<Json<SnapshotResource>, ApiError> {
    let id: SnapshotId = id
        .parse()
        .map_err(|_| ApiError::not_found("snapshot.not_found"))?;
    let manifest = state
        .snapshots
        .manifest(id)
        .await?
        .ok_or_else(|| ApiError::not_found("snapshot.not_found"))?;
    Ok(Json(SnapshotResource::from((id, &manifest))))
}
