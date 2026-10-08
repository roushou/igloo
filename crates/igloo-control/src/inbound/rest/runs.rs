use axum::Json;
use axum::extract::{Path, State};
use igloo_api::problem::Problem;
use igloo_api::run::{CheckResource, CheckStatus, RunPhase as ApiPhase, RunResource};
use igloo_core::Entity;
use igloo_core::change::ChangeId;

use super::auth::Caller;
use super::{ApiError, ApiState};
use crate::app::AppError;
use crate::ci::{CheckOutcome, Run, RunId, RunOutcome, RunPhase};

/// Lists the runs of a change, one per revision, oldest first.
#[utoipa::path(
    get,
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

/// Gets a run.
#[utoipa::path(
    get,
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
    Ok(convert(run, warm_job))
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
