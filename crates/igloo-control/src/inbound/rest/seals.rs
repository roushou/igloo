use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use igloo_api::problem::Problem;
use igloo_api::seal::SealResource;
use igloo_core::sandbox::SandboxId;
use igloo_core::seal::{Seal, SealId};
use igloo_core::snapshot::{MediaType, SnapshotLayer};
use igloo_core::{Actor, Digest, SystemComponent, ValidationErrors};
use serde::Deserialize;

use super::auth::Caller;
use super::upload::Upload;
use super::{ApiError, ApiState};
use crate::app::{AppError, RequestContext};
use crate::inbound::BlobSignature;
use crate::platform::{CompleteSeal, CreateSeal, StoreBlob};
use crate::ports::IdGeneratorExt;

/// The query of a seal layer upload: its presigned signature and what the layer holds.
#[derive(Deserialize)]
pub(super) struct LayerUpload {
    expires: i64,
    signature: String,
    /// The layer's digest; the stored bytes must hash to it.
    digest: String,
    /// `full` when the layer holds the whole root file system; changes otherwise.
    #[serde(default)]
    layer: Option<String>,
}

/// Seals a running sandbox into a snapshot. The seal completes in the background; poll it with
/// `GET /v1/seals/{id}`.
#[utoipa::path(
    post,
    operation_id = "createSeal",
    path = "/v1/sandboxes/{id}/snapshot",
    tag = "sandboxes",
    params(("id" = String, Path)),
    responses(
        (status = 202, body = SealResource),
        (status = 404, body = Problem),
        (status = 409, body = Problem),
    )
)]
pub(super) async fn create(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<SealResource>), ApiError> {
    let sandbox: SandboxId = id
        .parse()
        .map_err(|_| ApiError::not_found("sandbox.not_found"))?;
    let seal = state.bus.dispatch(CreateSeal { sandbox }, context).await?;
    let seal = load(&state, seal).await?;
    Ok((StatusCode::ACCEPTED, Json(SealResource::from(&seal))))
}

/// Gets a seal.
#[utoipa::path(
    get,
    operation_id = "getSeal",
    path = "/v1/seals/{id}",
    tag = "sandboxes",
    params(("id" = String, Path)),
    responses((status = 200, body = SealResource), (status = 404, body = Problem))
)]
pub(super) async fn get(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
) -> Result<Json<SealResource>, ApiError> {
    let id: SealId = id
        .parse()
        .map_err(|_| ApiError::not_found("seal.not_found"))?;
    Ok(Json(SealResource::from(&load(&state, id).await?)))
}

/// Uploads the layer that completes a seal, as a tar archive. Authenticated only by the
/// presigned URL handed to the sandbox's worker.
#[utoipa::path(
    put,
    operation_id = "uploadSealLayer",
    path = "/v1/seals/{id}/layer",
    tag = "sandboxes",
    params(
        ("id" = String, Path),
        ("expires" = i64, Query, description = "Presigned URL expiry, Unix seconds"),
        ("signature" = String, Query, description = "Presigned URL signature"),
        ("digest" = String, Query, description = "The layer's digest, `blake3:<64 hex>`"),
        ("layer" = Option<String>, Query, description = "`full` for a whole root file system"),
    ),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses(
        (status = 204),
        (status = 403, body = Problem),
        (status = 404, body = Problem),
        (status = 409, body = Problem),
        (status = 413, body = Problem),
        (status = 422, body = Problem),
    )
)]
pub(super) async fn upload(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(upload): Query<LayerUpload>,
    Upload(data): Upload,
) -> Result<StatusCode, ApiError> {
    let seal: SealId = id
        .parse()
        .map_err(|_| ApiError::not_found("seal.not_found"))?;
    state
        .blob_urls
        .verify_seal_upload(
            seal,
            &BlobSignature {
                expires: upload.expires,
                signature: upload.signature,
            },
        )
        .map_err(ApiError::from)?;
    let digest = upload.digest.parse::<Digest>().map_err(|error| {
        AppError::Validation(ValidationErrors::single("digest", error.to_string()))
    })?;
    let context = || {
        RequestContext::new(
            Actor::System {
                component: SystemComponent::Gateway,
            },
            state.ids.next(),
        )
    };
    state
        .bus
        .dispatch(StoreBlob { digest, data }, context())
        .await?;
    let complete = CompleteSeal {
        seal,
        layer: SnapshotLayer::new(digest, MediaType::Tar),
        full: upload.layer.as_deref() == Some("full"),
    };
    state.bus.dispatch(complete, context()).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn load(state: &ApiState, id: SealId) -> Result<Seal, ApiError> {
    Ok(state
        .seals
        .load(id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("seal.not_found"))?
        .entity()
        .clone())
}
