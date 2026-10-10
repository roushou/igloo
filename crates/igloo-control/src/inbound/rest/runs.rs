use axum::Json;
use axum::extract::{Path, Query, State};
use igloo_api::problem::Problem;
use igloo_api::run::{CheckResource, CheckStatus, RunList, RunPhase as ApiPhase, RunResource};
use igloo_core::change::ChangeId;
use igloo_core::repo::RepoId;
use igloo_core::{Entity, ValidationErrors, Validator};

use super::auth::Caller;
use super::{ApiError, ApiState};
use crate::app::AppError;
use crate::ci::{CheckOutcome, Run, RunId, RunOutcome, RunPhase};

/// Lists the runs of a change, one per revision, oldest first.
#[utoipa::path(
    get,
    operation_id = "listChangeRuns",
    path = "/v1/changes/{id}/runs",
    tag = "changes",
    params(("id" = String, Path)),
    responses((status = 200, body = Vec<RunResource>))
)]
pub(super) async fn of_change(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
) -> Result<Json<Vec<RunResource>>, ApiError> {
    Ok(Json(runs_of_change(&state, &id).await?))
}

/// The runs of change `id`, one per revision, oldest first.
pub(crate) async fn runs_of_change(
    state: &ApiState,
    id: &str,
) -> Result<Vec<RunResource>, ApiError> {
    let change: ChangeId = id
        .parse()
        .map_err(|_| ApiError::not_found("change.not_found"))?;
    let runs = state.runs.of_change(change).await.map_err(AppError::from)?;
    let mut resources = Vec::with_capacity(runs.len());
    for run in &runs {
        resources.push(resource(state, run).await?);
    }
    Ok(resources)
}

/// Lists a repository's runs, newest first, a page at a time.
#[utoipa::path(
    get,
    operation_id = "listRepoRuns",
    path = "/v1/repos/{id}/runs",
    tag = "changes",
    params(
        ("id" = String, Path),
        ("cursor" = Option<String>, Query, description = "`next_cursor` of the previous page"),
        ("limit" = Option<u32>, Query, description = "Page size, 1 to 200; 50 when omitted"),
    ),
    responses((status = 200, body = RunList), (status = 404, body = Problem), (status = 422, body = Problem))
)]
pub(super) async fn of_repo(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
    Query(params): Query<Vec<(String, String)>>,
) -> Result<Json<RunList>, ApiError> {
    let page = Page::try_from(params).map_err(AppError::from)?;
    let repo: RepoId = id
        .parse()
        .map_err(|_| ApiError::not_found("repo.not_found"))?;
    state
        .repos
        .get(repo)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("repo.not_found"))?;
    let runs = state.runs.of_repo(repo).await.map_err(AppError::from)?;
    let mut rest = runs
        .iter()
        .rev()
        .filter(|run| page.cursor.is_none_or(|cursor| run.id() < cursor));
    let mut items = Vec::new();
    for run in rest.by_ref().take(page.limit) {
        items.push(resource(&state, run).await?);
    }
    let next_cursor = rest.next().and(items.last()).map(|last| last.id.clone());
    Ok(Json(RunList::new(items, next_cursor)))
}

/// List parameters, validated.
struct Page {
    cursor: Option<RunId>,
    limit: usize,
}

impl Page {
    const DEFAULT_LIMIT: usize = 50;
    const MAX_LIMIT: usize = 200;
}

impl TryFrom<Vec<(String, String)>> for Page {
    type Error = ValidationErrors;

    fn try_from(params: Vec<(String, String)>) -> Result<Self, Self::Error> {
        let mut cursor = Ok(None);
        let mut limit = Ok(Self::DEFAULT_LIMIT);
        for (key, value) in params {
            match key.as_str() {
                "cursor" => {
                    cursor = value
                        .parse::<RunId>()
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
        let (cursor, limit) = Validator::new()
            .field("cursor", cursor)
            .field("limit", limit)
            .finish()?;
        Ok(Self { cursor, limit })
    }
}

/// Gets a run.
#[utoipa::path(
    get,
    operation_id = "getRun",
    path = "/v1/runs/{id}",
    tag = "changes",
    params(("id" = String, Path)),
    responses((status = 200, body = RunResource), (status = 404, body = Problem))
)]
pub(super) async fn get(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
) -> Result<Json<RunResource>, ApiError> {
    let id: RunId = id
        .parse()
        .map_err(|_| ApiError::not_found("run.not_found"))?;
    let run = state
        .runs
        .get(id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("run.not_found"))?;
    Ok(Json(resource(&state, &run).await?))
}

/// The run as a resource, with the job of its warm build when it did one.
async fn resource(state: &ApiState, run: &Run) -> Result<RunResource, ApiError> {
    let warm_job = match run.warm_build() {
        Some(build) => state
            .builds
            .get(build)
            .await
            .map_err(AppError::from)?
            .and_then(|build| build.job()),
        None => None,
    };
    let mut resource = convert(run, warm_job);
    if run.outcome().is_some()
        && let Some(at) = state
            .timings
            .span(&run.id().to_string())
            .await
            .map_err(AppError::from)?
            .ended
    {
        resource = resource.with_ended_at(at);
    }
    if let Some(sandbox) = run.sandbox() {
        resource = resource.with_sandbox(sandbox.to_string());
    }
    for check in &mut resource.checks {
        let Some(job) = &check.job else { continue };
        let span = state.timings.span(job).await.map_err(AppError::from)?;
        check.started_at = span.started;
        check.ended_at = span.ended;
    }
    Ok(resource)
}

fn convert(run: &Run, warm_job: Option<igloo_core::job::JobId>) -> RunResource {
    let phase = match (run.phase(), run.outcome()) {
        (_, Some(RunOutcome::Passed)) => ApiPhase::Passed,
        (_, Some(RunOutcome::Failed)) => ApiPhase::Failed,
        (_, Some(RunOutcome::Errored { .. })) => ApiPhase::Errored,
        (RunPhase::Preparing, None) => ApiPhase::Preparing,
        (RunPhase::Warming, None) => ApiPhase::Warming,
        (RunPhase::Checking | RunPhase::Ended, None) => ApiPhase::Checking,
    };
    let checks = run
        .checks()
        .iter()
        .map(|check| {
            let status = match (&check.job, &check.outcome) {
                (None, _) => CheckStatus::Pending,
                (Some(_), None) => CheckStatus::Started,
                (Some(_), Some(CheckOutcome::Passed)) => CheckStatus::Passed,
                (Some(_), Some(CheckOutcome::Failed { .. })) => CheckStatus::Failed,
                (Some(_), Some(CheckOutcome::Errored { .. })) => CheckStatus::Errored,
            };
            let mut resource = CheckResource::new(check.spec.name.clone(), status);
            if let Some(job) = check.job {
                resource = resource.with_job(job.to_string());
            }
            match &check.outcome {
                Some(CheckOutcome::Failed { exit_code }) => resource.with_exit_code(*exit_code),
                Some(CheckOutcome::Errored { reason }) => resource.with_reason(reason.clone()),
                _ => resource,
            }
        })
        .collect();
    let mut resource = RunResource::new(
        run.id().to_string(),
        (run.change().to_string(), run.revision()),
        run.commit().to_string(),
        phase,
        run.started_at(),
    )
    .with_checks(checks);
    if let Some(job) = warm_job {
        resource = resource.with_warm_job(job.to_string());
    }
    if let Some(RunOutcome::Errored { reason }) = run.outcome() {
        resource = resource.with_error(reason.clone());
    }
    resource
}
