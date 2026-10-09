use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use igloo_api::job::{ExecRequest, JobResource};
use igloo_api::problem::Problem;
use igloo_api::sandbox::{CreateSandboxRequest, SandboxList, SandboxResource};
use igloo_core::sandbox::{Sandbox, SandboxId, SandboxSpec};
use igloo_core::{Entity, Labels, ValidationErrors, Validator};

use super::auth::Caller;
use super::{ApiError, ApiState};
use crate::app::AppError;
use crate::platform::{CreateSandbox, StopSandbox, SubmitJob};

type Created<T> = (StatusCode, [(header::HeaderName, String); 1], Json<T>);

/// Creates a sandbox from a registered snapshot.
#[utoipa::path(
    post,
    operation_id = "createSandbox",
    path = "/v1/sandboxes",
    tag = "sandboxes",
    request_body = CreateSandboxRequest,
    params(("Idempotency-Key" = Option<String>, Header, description = "Makes retries safe")),
    responses(
        (status = 201, body = SandboxResource),
        (status = 401, body = Problem),
        (status = 422, body = Problem),
    )
)]
pub(super) async fn create(
    State(state): State<ApiState>,
    Caller(context): Caller,
    request: Result<Json<CreateSandboxRequest>, JsonRejection>,
) -> Result<Created<SandboxResource>, ApiError> {
    let Json(request) = request?;
    let spec = SandboxSpec::try_from(request).map_err(AppError::from)?;
    let id = state.bus.dispatch(CreateSandbox { spec }, context).await?;
    let sandbox = load(&state, id).await?;
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, format!("/v1/sandboxes/{id}"))],
        Json(SandboxResource::from(&sandbox)),
    ))
}

/// Lists sandboxes, ordered by id, optionally filtered by labels (`label=key=value`, repeatable).
#[utoipa::path(
    get,
    operation_id = "listSandboxes",
    path = "/v1/sandboxes",
    tag = "sandboxes",
    params(
        ("label" = Option<Vec<String>>, Query, description = "`key=value`; every label must match"),
        ("cursor" = Option<String>, Query, description = "`next_cursor` of the previous page"),
        ("limit" = Option<u32>, Query, description = "Page size, 1 to 200; 50 when omitted"),
    ),
    responses((status = 200, body = SandboxList), (status = 422, body = Problem))
)]
pub(super) async fn list(
    State(state): State<ApiState>,
    _: Caller,
    Query(params): Query<Vec<(String, String)>>,
) -> Result<Json<SandboxList>, ApiError> {
    let page = Page::try_from(params).map_err(AppError::from)?;
    let matching = state
        .sandboxes
        .matching(&page.labels)
        .await
        .map_err(AppError::from)?;
    let mut rest = matching
        .iter()
        .filter(|sandbox| page.cursor.is_none_or(|cursor| sandbox.id() > cursor));
    let items: Vec<SandboxResource> = rest.by_ref().take(page.limit).map(Into::into).collect();
    let next_cursor = rest.next().and(items.last()).map(|last| last.id.clone());
    Ok(Json(SandboxList::new(items, next_cursor)))
}

/// Gets a sandbox.
#[utoipa::path(
    get,
    operation_id = "getSandbox",
    path = "/v1/sandboxes/{id}",
    tag = "sandboxes",
    params(("id" = String, Path)),
    responses((status = 200, body = SandboxResource), (status = 404, body = Problem))
)]
pub(super) async fn get(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
) -> Result<Json<SandboxResource>, ApiError> {
    let sandbox = load(&state, parse_id(&id)?).await?;
    Ok(Json(SandboxResource::from(&sandbox)))
}

/// Stops a sandbox. Idempotent.
#[utoipa::path(
    post,
    operation_id = "stopSandbox",
    path = "/v1/sandboxes/{id}/stop",
    tag = "sandboxes",
    params(("id" = String, Path)),
    responses((status = 202, body = SandboxResource), (status = 404, body = Problem))
)]
pub(super) async fn stop(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<SandboxResource>), ApiError> {
    let sandbox = parse_id(&id)?;
    state.bus.dispatch(StopSandbox { sandbox }, context).await?;
    let sandbox = load(&state, sandbox).await?;
    Ok((StatusCode::ACCEPTED, Json(SandboxResource::from(&sandbox))))
}

/// Runs a process in a sandbox as a job.
#[utoipa::path(
    post,
    operation_id = "execInSandbox",
    path = "/v1/sandboxes/{id}/exec",
    tag = "sandboxes",
    params(("id" = String, Path)),
    request_body = ExecRequest,
    responses(
        (status = 201, body = JobResource),
        (status = 404, body = Problem),
        (status = 409, body = Problem),
        (status = 422, body = Problem),
    )
)]
pub(super) async fn exec(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
    request: Result<Json<ExecRequest>, JsonRejection>,
) -> Result<Created<JobResource>, ApiError> {
    let sandbox = parse_id(&id)?;
    let Json(request) = request?;
    let spec = request.into_spec(sandbox).map_err(AppError::from)?;
    let job = state.bus.dispatch(SubmitJob { spec }, context).await?;
    let resource = super::jobs::load(&state, job).await?;
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, format!("/v1/jobs/{job}"))],
        Json(resource),
    ))
}

/// A malformed id names no sandbox.
fn parse_id(id: &str) -> Result<SandboxId, ApiError> {
    id.parse()
        .map_err(|_| ApiError::not_found("sandbox.not_found"))
}

async fn load(state: &ApiState, id: SandboxId) -> Result<Sandbox, ApiError> {
    state
        .sandboxes
        .get(id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("sandbox.not_found"))
}

/// List parameters, validated.
struct Page {
    labels: Labels,
    cursor: Option<SandboxId>,
    limit: usize,
}

impl Page {
    const DEFAULT_LIMIT: usize = 50;
    const MAX_LIMIT: usize = 200;
}

impl TryFrom<Vec<(String, String)>> for Page {
    type Error = ValidationErrors;

    fn try_from(params: Vec<(String, String)>) -> Result<Self, Self::Error> {
        let mut labels = Vec::new();
        let mut cursor = Ok(None);
        let mut limit = Ok(Self::DEFAULT_LIMIT);
        for (key, value) in params {
            match key.as_str() {
                "label" => labels.push(value),
                "cursor" => {
                    cursor = value
                        .parse::<SandboxId>()
                        .map(Some)
                        .map_err(|error| error.to_string());
                }
                "limit" => {
                    limit = value
                        .parse::<usize>()
                        .ok()
                        .filter(|limit| (1..=Self::MAX_LIMIT).contains(limit))
                        .ok_or_else(|| format!("must be 1 to {}", Self::MAX_LIMIT));
                }
                _ => {}
            }
        }
        let pairs: Result<Vec<(String, String)>, String> = labels
            .into_iter()
            .map(|label| {
                label
                    .split_once('=')
                    .map(|(key, value)| (key.to_owned(), value.to_owned()))
                    .ok_or_else(|| format!("{label:?} is not key=value"))
            })
            .collect();
        let (pairs, cursor, limit) = Validator::new()
            .field("label", pairs)
            .field("cursor", cursor)
            .field("limit", limit)
            .finish()?;
        let (labels,) = Validator::new()
            .nested("label", Labels::from_pairs(pairs))
            .finish()?;
        Ok(Self {
            labels,
            cursor,
            limit,
        })
    }
}
