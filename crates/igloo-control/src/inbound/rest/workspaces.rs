use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use igloo_api::problem::Problem;
use igloo_api::workspace::{CreateWorkspaceRequest, WorkspaceResource};
use igloo_core::repo::{BranchName, RepoId};
use igloo_core::workspace::{Workspace, WorkspaceId};
use igloo_core::{Entity, Resource as _, ValidationErrors};

use super::auth::Caller;
use super::{ApiError, ApiState};
use crate::app::AppError;
use crate::workspaces::{CreateWorkspace, DeleteWorkspace, StartWorkspace, StopWorkspace};

/// Opens a workspace on a branch of a repository for the caller. It starts at once: its sandbox
/// is forked from the repository's warm snapshot at the branch head.
#[utoipa::path(
    post,
    operation_id = "createWorkspace",
    path = "/v1/repos/{id}/workspaces",
    tag = "workspaces",
    params(("id" = String, Path)),
    request_body = CreateWorkspaceRequest,
    responses(
        (status = 201, body = WorkspaceResource),
        (status = 404, body = Problem),
        (status = 422, body = Problem),
    )
)]
pub(super) async fn create(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
    request: Result<Json<CreateWorkspaceRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<WorkspaceResource>), ApiError> {
    let Json(request) = request?;
    let repo: RepoId = id
        .parse()
        .map_err(|_| ApiError::not_found("repo.not_found"))?;
    let repo = state
        .repos
        .get(repo)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("repo.not_found"))?;
    let branch = match request.branch {
        Some(branch) => branch.parse::<BranchName>().map_err(|error| {
            AppError::Validation(ValidationErrors::single("branch", error.to_string()))
        })?,
        None => repo.default_branch().clone(),
    };
    let remote = state.repos.remote(&repo).await?;
    state
        .forge
        .fetch(&remote, &branch)
        .await
        .map_err(AppError::from)?;
    let command = CreateWorkspace {
        repo: Entity::id(&repo),
        branch,
    };
    let workspace = state.bus.dispatch(command, context).await?;
    Ok((StatusCode::CREATED, Json(load(&state, workspace).await?)))
}

/// Lists the caller's workspaces, oldest first.
#[utoipa::path(
    get,
    operation_id = "listWorkspaces",
    path = "/v1/workspaces",
    tag = "workspaces",
    responses((status = 200, body = Vec<WorkspaceResource>))
)]
pub(super) async fn list(
    State(state): State<ApiState>,
    Caller(context): Caller,
) -> Result<Json<Vec<WorkspaceResource>>, ApiError> {
    let owner = context.actor.principal();
    let workspaces = state.workspaces.all().await.map_err(AppError::from)?;
    Ok(Json(
        workspaces
            .iter()
            .filter(|workspace| owner == Some(workspace.spec().owner))
            .map(WorkspaceResource::from)
            .collect(),
    ))
}

/// Gets a workspace.
#[utoipa::path(
    get,
    operation_id = "getWorkspace",
    path = "/v1/workspaces/{id}",
    tag = "workspaces",
    params(("id" = String, Path)),
    responses((status = 200, body = WorkspaceResource), (status = 404, body = Problem))
)]
pub(super) async fn get(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceResource>, ApiError> {
    Ok(Json(load(&state, parse(&id)?).await?))
}

/// Starts a workspace: a sandbox resumes from what its last stop sealed. Starting a running
/// workspace changes nothing; a workspace that is stopping starts again once it has stopped.
#[utoipa::path(
    post,
    operation_id = "startWorkspace",
    path = "/v1/workspaces/{id}/start",
    tag = "workspaces",
    params(("id" = String, Path)),
    responses((status = 200, body = WorkspaceResource), (status = 404, body = Problem))
)]
pub(super) async fn start(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceResource>, ApiError> {
    let workspace = load_domain(&state, parse(&id)?).await?;
    let command = StartWorkspace {
        workspace: Entity::id(&workspace),
    };
    state.bus.dispatch(command, context).await?;
    Ok(Json(load(&state, Entity::id(&workspace)).await?))
}

/// Stops a workspace: its changes are sealed, then its sandbox stops. Stopping a stopped
/// workspace changes nothing.
#[utoipa::path(
    post,
    operation_id = "stopWorkspace",
    path = "/v1/workspaces/{id}/stop",
    tag = "workspaces",
    params(("id" = String, Path)),
    responses((status = 200, body = WorkspaceResource), (status = 404, body = Problem))
)]
pub(super) async fn stop(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceResource>, ApiError> {
    let workspace = load_domain(&state, parse(&id)?).await?;
    let command = StopWorkspace {
        workspace: Entity::id(&workspace),
    };
    state.bus.dispatch(command, context).await?;
    Ok(Json(load(&state, Entity::id(&workspace)).await?))
}

/// Deletes a workspace: its sandbox is stopped without sealing and its changes are dropped.
#[utoipa::path(
    delete,
    operation_id = "deleteWorkspace",
    path = "/v1/workspaces/{id}",
    tag = "workspaces",
    params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = Problem))
)]
pub(super) async fn delete(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let workspace = load_domain(&state, parse(&id)?).await?;
    let command = DeleteWorkspace {
        workspace: Entity::id(&workspace),
    };
    state.bus.dispatch(command, context).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn parse(id: &str) -> Result<WorkspaceId, ApiError> {
    id.parse()
        .map_err(|_| ApiError::not_found("workspace.not_found"))
}

async fn load_domain(state: &ApiState, id: WorkspaceId) -> Result<Workspace, ApiError> {
    state
        .workspaces
        .get(id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("workspace.not_found"))
}

async fn load(state: &ApiState, id: WorkspaceId) -> Result<WorkspaceResource, ApiError> {
    Ok(WorkspaceResource::from(&load_domain(state, id).await?))
}
