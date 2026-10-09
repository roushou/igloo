use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, JsonRejection};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use igloo_api::problem::Problem;
use igloo_api::repo::{
    CheckoutTarget, RegisterRepoRequest, RepoRegistration, RepoResource, RepoSnapshotRequest,
    RepoSnapshotResource, SecretList,
};
use igloo_core::ValidationErrors;
use igloo_core::repo::{RepoId, SecretName};

use super::auth::Caller;
use super::{ApiError, ApiState};
use crate::app::AppError;
use crate::platform::{DeleteSecret, RegisterRepo, SetSecret};
use crate::ports::SecretValue;

/// Registers a repository.
#[utoipa::path(
    post,
    operation_id = "registerRepo",
    path = "/v1/repos",
    tag = "repos",
    request_body = RegisterRepoRequest,
    responses(
        (status = 201, body = RepoResource),
        (status = 409, body = Problem),
        (status = 422, body = Problem),
    )
)]
pub(super) async fn register(
    State(state): State<ApiState>,
    Caller(context): Caller,
    request: Result<Json<RegisterRepoRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<RepoResource>), ApiError> {
    let Json(request) = request?;
    let registration = RepoRegistration::try_from(request).map_err(AppError::from)?;
    let command = RegisterRepo {
        location: registration.location,
        default_branch: registration.default_branch,
        token: registration.token,
    };
    let id = state.bus.dispatch(command, context).await?;
    let repo = load(&state, id).await?;
    Ok((StatusCode::CREATED, Json(repo)))
}

/// Lists repositories, oldest first.
#[utoipa::path(
    get,
    operation_id = "listRepos",
    path = "/v1/repos",
    tag = "repos",
    responses((status = 200, body = Vec<RepoResource>))
)]
pub(super) async fn list(
    State(state): State<ApiState>,
    _: Caller,
) -> Result<Json<Vec<RepoResource>>, ApiError> {
    Ok(Json(list_repos(&state).await?))
}

/// Every registered repository.
pub(crate) async fn list_repos(state: &ApiState) -> Result<Vec<RepoResource>, ApiError> {
    let repos = state.repos.all().await.map_err(AppError::from)?;
    Ok(repos.iter().map(RepoResource::from).collect())
}

/// Gets a repository.
#[utoipa::path(
    get,
    operation_id = "getRepo",
    path = "/v1/repos/{id}",
    tag = "repos",
    params(("id" = String, Path)),
    responses((status = 200, body = RepoResource), (status = 404, body = Problem))
)]
pub(super) async fn get(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
) -> Result<Json<RepoResource>, ApiError> {
    Ok(Json(load(&state, parse_id(&id)?).await?))
}

/// Snapshots a repository at a commit: the checkout under `/workspace`, with a shallow `.git`,
/// over an optional base snapshot. A branch is fetched from the forge first.
#[utoipa::path(
    post,
    operation_id = "snapshotRepo",
    path = "/v1/repos/{id}/snapshots",
    tag = "repos",
    params(("id" = String, Path)),
    request_body = RepoSnapshotRequest,
    responses(
        (status = 201, body = RepoSnapshotResource),
        (status = 404, body = Problem),
        (status = 422, body = Problem),
    )
)]
pub(super) async fn snapshot(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
    request: Result<Json<RepoSnapshotRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<RepoSnapshotResource>), ApiError> {
    let Json(request) = request?;
    let (target, base) = request.target().map_err(AppError::from)?;
    let id = parse_id(&id)?;
    let repo = state
        .repos
        .get(id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("repo.not_found"))?;
    let remote = state.repos.remote(&repo).await?;
    let commit = match target {
        CheckoutTarget::Branch(branch) => state
            .forge
            .fetch(&remote, &branch)
            .await
            .map_err(AppError::from)?,
        CheckoutTarget::Commit(commit) => {
            state
                .forge
                .fetch(&remote, repo.default_branch())
                .await
                .map_err(AppError::from)?;
            commit
        }
    };
    let snapshot = state
        .repo_snapshots
        .checkout(id, &commit, base, None)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(RepoSnapshotResource::new(
            snapshot.to_string(),
            commit.to_string(),
        )),
    ))
}

/// Lists the names of a repository's secrets; values are never returned.
#[utoipa::path(
    get,
    operation_id = "listRepoSecrets",
    path = "/v1/repos/{id}/secrets",
    tag = "repos",
    params(("id" = String, Path)),
    responses((status = 200, body = SecretList), (status = 404, body = Problem))
)]
pub(super) async fn list_secrets(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
) -> Result<Json<SecretList>, ApiError> {
    let id = parse_id(&id)?;
    load(&state, id).await?;
    let names = state.secrets.names(id).await.map_err(AppError::from)?;
    Ok(Json(SecretList::from(names)))
}

/// Sets or replaces a secret. The body is its value, as text.
#[utoipa::path(
    put,
    operation_id = "setRepoSecret",
    path = "/v1/repos/{id}/secrets/{name}",
    tag = "repos",
    params(("id" = String, Path), ("name" = String, Path, description = "`[A-Z_][A-Z0-9_]*`")),
    request_body(content = String, content_type = "text/plain"),
    responses((status = 204), (status = 404, body = Problem), (status = 422, body = Problem))
)]
pub(super) async fn set_secret(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path((id, name)): Path<(String, String)>,
    body: Result<Bytes, BytesRejection>,
) -> Result<StatusCode, ApiError> {
    let repo = parse_id(&id)?;
    let name = parse_name(&name)?;
    let value = String::from_utf8(body?.to_vec())
        .ok()
        .and_then(|value| SecretValue::try_from(value).ok())
        .ok_or_else(|| {
            AppError::Validation(ValidationErrors::single(
                "value",
                "must be UTF-8 text of at most 64 KiB without NUL",
            ))
        })?;
    let command = SetSecret { repo, name, value };
    state.bus.dispatch(command, context).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes a secret.
#[utoipa::path(
    delete,
    operation_id = "deleteRepoSecret",
    path = "/v1/repos/{id}/secrets/{name}",
    tag = "repos",
    params(("id" = String, Path), ("name" = String, Path)),
    responses((status = 204), (status = 404, body = Problem))
)]
pub(super) async fn delete_secret(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path((id, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let command = DeleteSecret {
        repo: parse_id(&id)?,
        name: parse_name(&name)?,
    };
    state.bus.dispatch(command, context).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn parse_id(id: &str) -> Result<RepoId, ApiError> {
    id.parse()
        .map_err(|_| ApiError::not_found("repo.not_found"))
}

fn parse_name(name: &str) -> Result<SecretName, ApiError> {
    name.parse::<SecretName>().map_err(|error| {
        AppError::Validation(ValidationErrors::single("name", error.to_string())).into()
    })
}

async fn load(state: &ApiState, id: RepoId) -> Result<RepoResource, ApiError> {
    let repo = state
        .repos
        .get(id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("repo.not_found"))?;
    Ok(RepoResource::from(&repo))
}
