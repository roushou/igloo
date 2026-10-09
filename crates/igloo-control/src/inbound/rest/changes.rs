use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use igloo_api::change::{ApproveRequest, ChangeResource, CommentRequest, OpenChangeRequest};
use igloo_api::problem::Problem;
use igloo_core::change::ChangeId;
use igloo_core::repo::RepoId;

use super::auth::Caller;
use super::{ApiError, ApiState};
use crate::app::{AppError, RequestContext};
use crate::platform::{
    ApproveChange, CloseChange, CommentChange, OpenChange, RequestChanges, ReviseChange,
};

/// Opens a change proposing a branch pushed to the forge; its head is the first revision.
#[utoipa::path(
    post,
    path = "/v1/repos/{id}/changes",
    tag = "changes",
    params(("id" = String, Path)),
    request_body = OpenChangeRequest,
    responses(
        (status = 201, body = ChangeResource),
        (status = 404, body = Problem),
        (status = 409, body = Problem),
        (status = 422, body = Problem),
    )
)]
pub(super) async fn open(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
    request: Result<Json<OpenChangeRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ChangeResource>), ApiError> {
    let Json(request) = request?;
    let source = request.source().map_err(AppError::from)?;
    let repo: RepoId = id
        .parse()
        .map_err(|_| ApiError::not_found("repo.not_found"))?;
    let found = state
        .repos
        .get(repo)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("repo.not_found"))?;
    let heads = state.change_heads.of(&found, &source).await?;
    let command = OpenChange {
        repo,
        source,
        title: request.title,
        heads,
    };
    let change = state.bus.dispatch(command, context).await?;
    Ok((StatusCode::CREATED, Json(load(&state, change).await?)))
}

/// Lists a repository's changes, oldest first.
#[utoipa::path(
    get,
    path = "/v1/repos/{id}/changes",
    tag = "changes",
    params(("id" = String, Path)),
    responses((status = 200, body = Vec<ChangeResource>))
)]
pub(super) async fn list(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
) -> Result<Json<Vec<ChangeResource>>, ApiError> {
    let repo: RepoId = id
        .parse()
        .map_err(|_| ApiError::not_found("repo.not_found"))?;
    let changes = state.changes.of_repo(repo).await.map_err(AppError::from)?;
    Ok(Json(changes.iter().map(ChangeResource::from).collect()))
}

/// Gets a change.
#[utoipa::path(
    get,
    path = "/v1/changes/{id}",
    tag = "changes",
    params(("id" = String, Path)),
    responses((status = 200, body = ChangeResource), (status = 404, body = Problem))
)]
pub(super) async fn get(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
) -> Result<Json<ChangeResource>, ApiError> {
    Ok(Json(get_change(&state, &id).await?))
}

/// Change `id`.
pub(crate) async fn get_change(state: &ApiState, id: &str) -> Result<ChangeResource, ApiError> {
    load(state, parse_id(id)?).await
}

/// Records the source branch's current head as the next revision; an unchanged head records
/// nothing.
#[utoipa::path(
    post,
    path = "/v1/changes/{id}/revisions",
    tag = "changes",
    params(("id" = String, Path)),
    responses(
        (status = 200, body = ChangeResource),
        (status = 404, body = Problem),
        (status = 409, body = Problem),
    )
)]
pub(super) async fn revise(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
) -> Result<Json<ChangeResource>, ApiError> {
    let change = parse_id(&id)?;
    let found = state
        .changes
        .get(change)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("change.not_found"))?;
    let repo = state
        .repos
        .get(found.repo())
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("repo.not_found"))?;
    let heads = state.change_heads.of(&repo, found.source()).await?;
    state
        .bus
        .dispatch(ReviseChange { change, heads }, context)
        .await?;
    Ok(Json(load(&state, change).await?))
}

/// Approves a change's latest revision as the caller, who must be a human.
#[utoipa::path(
    post,
    path = "/v1/changes/{id}/approve",
    tag = "changes",
    params(("id" = String, Path)),
    request_body = ApproveRequest,
    responses(
        (status = 200, body = ChangeResource),
        (status = 404, body = Problem),
        (status = 409, body = Problem),
    )
)]
pub(super) async fn approve(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
    request: Result<Json<ApproveRequest>, JsonRejection>,
) -> Result<Json<ChangeResource>, ApiError> {
    let change = parse_id(&id)?;
    let request = request.map(|Json(request)| request).unwrap_or_default();
    let revision = match request.revision {
        Some(revision) => revision,
        None => {
            state
                .changes
                .get(change)
                .await
                .map_err(AppError::from)?
                .ok_or_else(|| ApiError::not_found("change.not_found"))?
                .latest()
                .number
        }
    };
    state
        .bus
        .dispatch(ApproveChange { change, revision }, context)
        .await?;
    Ok(Json(load(&state, change).await?))
}

/// Comments on a revision of a change, the latest by default, optionally on a line of a file.
#[utoipa::path(
    post,
    path = "/v1/changes/{id}/comments",
    tag = "changes",
    params(("id" = String, Path)),
    request_body = CommentRequest,
    responses(
        (status = 201, body = ChangeResource),
        (status = 404, body = Problem),
        (status = 409, body = Problem),
        (status = 422, body = Problem),
    )
)]
pub(super) async fn comment(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
    request: Result<Json<CommentRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ChangeResource>), ApiError> {
    let Json(request) = request?;
    let change = add_comment(&state, context, &id, request).await?;
    Ok((StatusCode::CREATED, Json(change)))
}

/// Comments on change `id` as the sender of `context`.
pub(crate) async fn add_comment(
    state: &ApiState,
    context: RequestContext,
    id: &str,
    request: CommentRequest,
) -> Result<ChangeResource, ApiError> {
    let change = parse_id(id)?;
    let command = CommentChange {
        change,
        revision: request.revision,
        body: request.body,
        path: request.path,
        line: request.line,
    };
    state.bus.dispatch(command, context).await?;
    load(state, change).await
}

/// Asks for another revision, carrying the comments made since the previous request. A change
/// made by a task sends them to its agent as its next turn.
#[utoipa::path(
    post,
    path = "/v1/changes/{id}/request-changes",
    tag = "changes",
    params(("id" = String, Path)),
    responses(
        (status = 200, body = ChangeResource),
        (status = 404, body = Problem),
        (status = 409, body = Problem),
    )
)]
pub(super) async fn request_changes(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
) -> Result<Json<ChangeResource>, ApiError> {
    Ok(Json(ask_for_changes(&state, context, &id).await?))
}

/// Asks for another revision of change `id` as the sender of `context`.
pub(crate) async fn ask_for_changes(
    state: &ApiState,
    context: RequestContext,
    id: &str,
) -> Result<ChangeResource, ApiError> {
    let change = parse_id(id)?;
    state
        .bus
        .dispatch(RequestChanges { change }, context)
        .await?;
    load(state, change).await
}

/// Merges a change: its latest revision's checks must have passed, it must be based on the
/// target branch's head, and changes to protected paths need a human approval. Igloo pushes
/// the revision to the target branch as one squashed commit naming the change.
#[utoipa::path(
    post,
    path = "/v1/changes/{id}/merge",
    tag = "changes",
    params(("id" = String, Path)),
    responses(
        (status = 200, body = ChangeResource),
        (status = 404, body = Problem),
        (status = 409, body = Problem),
    )
)]
pub(super) async fn merge(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
) -> Result<Json<ChangeResource>, ApiError> {
    let id = parse_id(&id)?;
    let change = state
        .changes
        .get(id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("change.not_found"))?;
    state.merger.merge(&change, context).await?;
    Ok(Json(load(&state, id).await?))
}

/// Closes a change without merging.
#[utoipa::path(
    post,
    path = "/v1/changes/{id}/close",
    tag = "changes",
    params(("id" = String, Path)),
    responses(
        (status = 200, body = ChangeResource),
        (status = 404, body = Problem),
        (status = 409, body = Problem),
    )
)]
pub(super) async fn close(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
) -> Result<Json<ChangeResource>, ApiError> {
    let change = parse_id(&id)?;
    state.bus.dispatch(CloseChange { change }, context).await?;
    Ok(Json(load(&state, change).await?))
}

fn parse_id(id: &str) -> Result<ChangeId, ApiError> {
    id.parse()
        .map_err(|_| ApiError::not_found("change.not_found"))
}

async fn load(state: &ApiState, id: ChangeId) -> Result<ChangeResource, ApiError> {
    let change = state
        .changes
        .get(id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("change.not_found"))?;
    Ok(ChangeResource::from(&change))
}
