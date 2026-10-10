use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use igloo_api::change::{
    ApprovalNeed, ApprovalState, ApproveRequest, ChangePhase, ChangeResource, ChecksState,
    CommentRequest, DiffQuery, DiffResource, FileChangeStatus, FileDiffResource, MergeReadiness,
    OpenChangeRequest,
};
use igloo_api::list::ListOrder;
use igloo_api::list::PhaseFilter;
use igloo_api::problem::Problem;
use igloo_core::Entity;
use igloo_core::change::{Change, ChangeId};
use igloo_core::repo::RepoId;

use super::auth::Caller;
use super::{ApiError, ApiState};
use crate::app::{AppError, RequestContext};
use crate::ci::{self, Readiness};
use crate::platform::{
    ApproveChange, CloseChange, CommentChange, OpenChange, RequestChanges, ReviseChange,
};
use crate::ports::{ChangedFile, FileStatus};

/// Opens a change proposing a branch pushed to the forge; its head is the first revision.
#[utoipa::path(
    post,
    operation_id = "openChange",
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

/// Lists a repository's changes, oldest first unless `order=newest`, optionally only those at
/// the given phases. Open changes carry their merge readiness.
#[utoipa::path(
    get,
    operation_id = "listRepoChanges",
    path = "/v1/repos/{id}/changes",
    tag = "changes",
    params(
        ("id" = String, Path),
        ("phase" = Option<Vec<ChangePhase>>, Query, description = "Only changes at this phase; repeatable"),
        ("order" = Option<ListOrder>, Query, description = "`newest` lists the newest first"),
    ),
    responses((status = 200, body = Vec<ChangeResource>), (status = 422, body = Problem))
)]
pub(super) async fn list(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
    Query(params): Query<Vec<(String, String)>>,
) -> Result<Json<Vec<ChangeResource>>, ApiError> {
    let filter = PhaseFilter::<ChangePhase>::try_from(params).map_err(AppError::from)?;
    let repo: RepoId = id
        .parse()
        .map_err(|_| ApiError::not_found("repo.not_found"))?;
    let changes = state.changes.of_repo(repo).await.map_err(AppError::from)?;
    let changes = filter.apply(changes, |change| ChangeResource::from(change).phase);
    let mut resources = Vec::with_capacity(changes.len());
    for change in &changes {
        resources.push(resource(&state, change).await?);
    }
    Ok(Json(resources))
}

/// Gets a change.
#[utoipa::path(
    get,
    operation_id = "getChange",
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

/// What a revision of a change changes: the files that differ between the revision's base and
/// head in the repository's mirror, each with its unified patch as git prints it. A patch over
/// 256 KiB is left out and its file marked truncated.
#[utoipa::path(
    get,
    operation_id = "getChangeDiff",
    path = "/v1/changes/{id}/diff",
    tag = "changes",
    params(("id" = String, Path), DiffQuery),
    responses((status = 200, body = DiffResource), (status = 404, body = Problem))
)]
pub(super) async fn diff(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
    Query(query): Query<DiffQuery>,
) -> Result<Json<DiffResource>, ApiError> {
    let change = state
        .changes
        .get(parse_id(&id)?)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("change.not_found"))?;
    let revision = match query.revision {
        None => change.latest(),
        Some(number) => change
            .revisions()
            .find(|revision| revision.number == number)
            .ok_or_else(|| ApiError::not_found("change.revision_not_found"))?,
    };
    let files = state
        .forge
        .diff(change.repo(), &revision.base, &revision.head)
        .await
        .map_err(AppError::from)?;
    let files = files.into_iter().map(FileDiffResource::from).collect();
    Ok(Json(DiffResource::new(revision, files)))
}

impl From<ChangedFile> for FileDiffResource {
    fn from(file: ChangedFile) -> Self {
        let status = match file.status {
            FileStatus::Added => FileChangeStatus::Added,
            FileStatus::Modified => FileChangeStatus::Modified,
            FileStatus::Deleted => FileChangeStatus::Deleted,
            FileStatus::Renamed => FileChangeStatus::Renamed,
        };
        let resource = Self::new(
            file.path,
            file.previous_path,
            status,
            file.additions,
            file.deletions,
        );
        if file.binary {
            resource.binary()
        } else {
            resource.with_patch(file.patch)
        }
    }
}

/// Change `id`.
pub(crate) async fn get_change(state: &ApiState, id: &str) -> Result<ChangeResource, ApiError> {
    load(state, parse_id(id)?).await
}

/// Records the source branch's current head as the next revision; an unchanged head records
/// nothing.
#[utoipa::path(
    post,
    operation_id = "reviseChange",
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
    operation_id = "approveChange",
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
    operation_id = "commentOnChange",
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
    operation_id = "requestChanges",
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
    operation_id = "mergeChange",
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
    operation_id = "closeChange",
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
    resource(state, &change).await
}

/// The change as a resource; an open change carries its merge readiness.
async fn resource(state: &ApiState, change: &Change) -> Result<ChangeResource, ApiError> {
    let mut resource = ChangeResource::from(change);
    if resource.phase != ChangePhase::Open {
        let at = state
            .timings
            .span(&change.id().to_string())
            .await
            .map_err(AppError::from)?
            .ended;
        if let Some(at) = at {
            resource = match resource.phase {
                ChangePhase::Merged => resource.with_merged_at(at),
                _ => resource.with_closed_at(at),
            };
        }
        return Ok(resource);
    }
    let readiness = state.merger.readiness(change).await?;
    Ok(resource.with_readiness(MergeReadiness::from(&readiness)))
}

impl From<&Readiness> for MergeReadiness {
    fn from(readiness: &Readiness) -> Self {
        let checks = match readiness.checks {
            ci::ChecksState::Passed => ChecksState::Passed,
            ci::ChecksState::Failed => ChecksState::Failed,
            ci::ChecksState::Running => ChecksState::Running,
            ci::ChecksState::Missing => ChecksState::Missing,
        };
        let state = if readiness.protected.is_empty() {
            ApprovalState::NotRequired
        } else if readiness.approved {
            ApprovalState::Given
        } else {
            ApprovalState::Required
        };
        Self::new(
            checks,
            ApprovalNeed::new(state, readiness.protected.clone()),
            readiness.fast_forward,
        )
    }
}
