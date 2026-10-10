use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use igloo_api::list::{ListOrder, PhaseFilter};
use igloo_api::problem::Problem;
use igloo_api::task::{
    CreateTaskRequest, TakeoverPhase as ApiTakeover, TakeoverResource, TaskPhase as ApiPhase,
    TaskResource, TranscriptEntry as ApiEntry, TranscriptItem, TranscriptResource, TurnResource,
    TurnStatus,
};
use igloo_core::repo::RepoId;
use igloo_core::{Entity, ValidationErrors};

use super::auth::Caller;
use super::{ApiError, ApiState};
use crate::agents::{
    CancelTask, CreateTask, Entry, HandBackTask, TakeOverTask, TakeoverPhase, Task, TaskEnd,
    TaskId, TaskPhase, ToolName,
};
use crate::app::{AppError, RequestContext};

/// Creates a task: the agent starts from the head of the repository's default branch.
#[utoipa::path(
    post,
    operation_id = "createTask",
    path = "/v1/repos/{id}/tasks",
    tag = "tasks",
    params(("id" = String, Path)),
    request_body = CreateTaskRequest,
    responses(
        (status = 201, body = TaskResource),
        (status = 404, body = Problem),
        (status = 422, body = Problem),
    )
)]
pub(super) async fn create(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
    request: Result<Json<CreateTaskRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<TaskResource>), ApiError> {
    let Json(request) = request?;
    let task = create_task(&state, context, &id, request).await?;
    Ok((StatusCode::CREATED, Json(task)))
}

/// Creates a task on repository `repo` as the sender of `context`.
pub(crate) async fn create_task(
    state: &ApiState,
    context: RequestContext,
    repo: &str,
    request: CreateTaskRequest,
) -> Result<TaskResource, ApiError> {
    let repo: RepoId = repo
        .parse()
        .map_err(|_| ApiError::not_found("repo.not_found"))?;
    let tool = request
        .tool
        .map(|tool| tool.parse::<ToolName>())
        .transpose()
        .map_err(|error| {
            AppError::Validation(ValidationErrors::single("tool", error.to_string()))
        })?;
    let command = CreateTask {
        repo,
        goal: request.goal,
        tool,
    };
    let task = state.bus.dispatch(command, context).await?;
    load(state, task).await
}

/// Lists a repository's tasks, oldest first unless `order=newest`, optionally only those at the
/// given phases.
#[utoipa::path(
    get,
    operation_id = "listRepoTasks",
    path = "/v1/repos/{id}/tasks",
    tag = "tasks",
    params(
        ("id" = String, Path),
        ("phase" = Option<Vec<ApiPhase>>, Query, description = "Only tasks at this phase; repeatable"),
        ("order" = Option<ListOrder>, Query, description = "`newest` lists the newest first"),
    ),
    responses((status = 200, body = Vec<TaskResource>), (status = 422, body = Problem))
)]
pub(super) async fn list(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
    Query(params): Query<Vec<(String, String)>>,
) -> Result<Json<Vec<TaskResource>>, ApiError> {
    let filter = PhaseFilter::try_from(params).map_err(AppError::from)?;
    Ok(Json(list_tasks(&state, &id, &filter).await?))
}

/// The tasks of repository `repo` that `filter` selects, in its order.
pub(crate) async fn list_tasks(
    state: &ApiState,
    repo: &str,
    filter: &PhaseFilter<ApiPhase>,
) -> Result<Vec<TaskResource>, ApiError> {
    let repo: RepoId = repo
        .parse()
        .map_err(|_| ApiError::not_found("repo.not_found"))?;
    let tasks = state.tasks.of_repo(repo).await.map_err(AppError::from)?;
    Ok(filter.apply(tasks.iter().map(resource).collect(), |task| task.phase))
}

/// Gets a task.
#[utoipa::path(
    get,
    operation_id = "getTask",
    path = "/v1/tasks/{id}",
    tag = "tasks",
    params(("id" = String, Path)),
    responses((status = 200, body = TaskResource), (status = 404, body = Problem))
)]
pub(super) async fn get(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
) -> Result<Json<TaskResource>, ApiError> {
    Ok(Json(get_task(&state, &id).await?))
}

/// Cancels a task and stops its sandbox. Cancelling an ended task changes nothing, except that
/// a task that failed collecting its commits stops the sandbox it kept for recovery.
#[utoipa::path(
    post,
    operation_id = "cancelTask",
    path = "/v1/tasks/{id}/cancel",
    tag = "tasks",
    params(("id" = String, Path)),
    responses((status = 200, body = TaskResource), (status = 404, body = Problem))
)]
pub(super) async fn cancel(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
) -> Result<Json<TaskResource>, ApiError> {
    Ok(Json(cancel_task(&state, context, &id).await?))
}

/// Cancels task `id` as the sender of `context`.
pub(crate) async fn cancel_task(
    state: &ApiState,
    context: RequestContext,
    id: &str,
) -> Result<TaskResource, ApiError> {
    let task = parse(id)?;
    state.bus.dispatch(CancelTask { task }, context).await?;
    load(state, task).await
}

/// Takes a task over for the caller, who must be a person: the caller's terminal in the task's
/// sandbox (`GET /v1/sandboxes/{id}/terminal?mode=read_write`) becomes writable and no turn
/// starts until they hand the task back. A running turn is not interrupted. The sandbox keeps
/// running until the task ends. Taking over a task the caller already holds changes nothing.
#[utoipa::path(
    post,
    operation_id = "takeOverTask",
    path = "/v1/tasks/{id}/take-over",
    tag = "tasks",
    params(("id" = String, Path)),
    responses(
        (status = 200, body = TaskResource),
        (status = 404, body = Problem),
        (status = 409, body = Problem),
    )
)]
pub(super) async fn take_over(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
) -> Result<Json<TaskResource>, ApiError> {
    Ok(Json(take_over_task(&state, context, &id).await?))
}

/// Takes task `id` over as the sender of `context`.
pub(crate) async fn take_over_task(
    state: &ApiState,
    context: RequestContext,
    id: &str,
) -> Result<TaskResource, ApiError> {
    let task = parse(id)?;
    state.bus.dispatch(TakeOverTask { task }, context).await?;
    load(state, task).await
}

/// Hands a task back, as any person (so a forgotten take-over can be released), once its last
/// turn ended and its commits became a revision (until then the request is refused with
/// `task.turn_running`). The sandbox is read for what the person changed, without touching it, and the task's next turn is asked
/// about the commits they added and the changes they left uncommitted. Uncommitted work stays in
/// place. Handing back a task already being handed back changes nothing.
#[utoipa::path(
    post,
    operation_id = "handBackTask",
    path = "/v1/tasks/{id}/hand-back",
    tag = "tasks",
    params(("id" = String, Path)),
    responses(
        (status = 200, body = TaskResource),
        (status = 404, body = Problem),
        (status = 409, body = Problem),
    )
)]
pub(super) async fn hand_back(
    State(state): State<ApiState>,
    Caller(context): Caller,
    Path(id): Path<String>,
) -> Result<Json<TaskResource>, ApiError> {
    Ok(Json(hand_back_task(&state, context, &id).await?))
}

/// Hands task `id` back as the sender of `context`.
pub(crate) async fn hand_back_task(
    state: &ApiState,
    context: RequestContext,
    id: &str,
) -> Result<TaskResource, ApiError> {
    let task = parse(id)?;
    state.bus.dispatch(HandBackTask { task }, context).await?;
    load(state, task).await
}

/// Where a transcript is read from.
#[derive(serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(super) struct TranscriptQuery {
    /// The first position to return; 0 when absent.
    #[serde(default)]
    after: u32,
}

/// A task's transcript from a position on: what its tool said and did, turn by turn, read from
/// its output with secrets masked. Poll with `after` set to the last response's `next`.
#[utoipa::path(
    get,
    operation_id = "getTaskTranscript",
    path = "/v1/tasks/{id}/transcript",
    tag = "tasks",
    params(("id" = String, Path), TranscriptQuery),
    responses((status = 200, body = TranscriptResource), (status = 404, body = Problem))
)]
pub(super) async fn transcript(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
    Query(query): Query<TranscriptQuery>,
) -> Result<Json<TranscriptResource>, ApiError> {
    Ok(Json(read_transcript(&state, &id, query.after).await?))
}

/// Task `id`'s transcript from position `after` on.
pub(crate) async fn read_transcript(
    state: &ApiState,
    id: &str,
    after: u32,
) -> Result<TranscriptResource, ApiError> {
    let task = state
        .tasks
        .get(parse(id)?)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("task.not_found"))?;
    let all = state.transcripts.of(&task).await.map_err(AppError::from)?;
    let next = u32::try_from(all.len()).unwrap_or(u32::MAX);
    let entries = all
        .into_iter()
        .zip(0..)
        .skip(usize::try_from(after).unwrap_or(usize::MAX))
        .map(|(entry, position)| ApiEntry::new(position, entry.turn, item(entry.entry)))
        .collect();
    let idle = task.phase() != TaskPhase::Working;
    Ok(TranscriptResource::new(entries, next, idle))
}

fn item(entry: Entry) -> TranscriptItem {
    match entry {
        Entry::Message { text } => TranscriptItem::Message { text },
        Entry::ToolCall { id, name, input } => TranscriptItem::ToolCall { id, name, input },
        Entry::ToolResult {
            id,
            output,
            is_error,
        } => TranscriptItem::ToolResult {
            id,
            output,
            is_error,
        },
        Entry::Output { text } => TranscriptItem::Output { text },
        Entry::Error { text } => TranscriptItem::Error { text },
    }
}

fn parse(id: &str) -> Result<TaskId, ApiError> {
    id.parse()
        .map_err(|_| ApiError::not_found("task.not_found"))
}

/// Task `id`.
pub(crate) async fn get_task(state: &ApiState, id: &str) -> Result<TaskResource, ApiError> {
    load(state, parse(id)?).await
}

async fn load(state: &ApiState, id: TaskId) -> Result<TaskResource, ApiError> {
    let task = state
        .tasks
        .get(id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("task.not_found"))?;
    Ok(resource(&task))
}

fn resource(task: &Task) -> TaskResource {
    let phase = match (task.phase(), task.end()) {
        (_, Some(TaskEnd::Done)) => ApiPhase::Done,
        (_, Some(TaskEnd::Failed { .. })) => ApiPhase::Failed,
        (_, Some(TaskEnd::Cancelled)) => ApiPhase::Cancelled,
        (TaskPhase::Preparing | TaskPhase::Ended, None) => ApiPhase::Preparing,
        (TaskPhase::Working, None) => ApiPhase::Working,
        (TaskPhase::AwaitingReview, None) => ApiPhase::AwaitingReview,
    };
    let turns = task
        .turns()
        .iter()
        .zip(1..)
        .map(|(turn, number)| {
            let status = match turn.ending {
                None => TurnStatus::Running,
                Some(ending) if ending.succeeded() => TurnStatus::Succeeded,
                Some(_) => TurnStatus::Failed,
            };
            let resource =
                TurnResource::new(number, turn.job.to_string(), turn.prompt.clone(), status);
            match turn.ending {
                Some(ending) if !ending.succeeded() => resource.with_reason(ending.to_string()),
                _ => resource,
            }
        })
        .collect();
    let mut resource = TaskResource::new(
        task.id().to_string(),
        task.repo().to_string(),
        task.goal().to_owned(),
        phase,
        task.created_at(),
    )
    .with_turns(turns);
    if let Some(prepared) = task.settings() {
        resource = resource.with_settings(prepared.tool.to_string(), prepared.commit.to_string());
    }
    if let Some(sandbox) = task.sandbox() {
        resource = resource.with_sandbox(sandbox.to_string());
    }
    if let Some(change) = task.change() {
        resource = resource.with_change(change.to_string());
    }
    if let (Some(by), Some(phase)) = (task.taken_over_by(), task.takeover()) {
        let phase = match phase {
            TakeoverPhase::Waiting => ApiTakeover::Waiting,
            TakeoverPhase::Paused => ApiTakeover::Paused,
            TakeoverPhase::HandingBack => ApiTakeover::HandingBack,
        };
        resource = resource.with_takeover(TakeoverResource::new(by.to_string(), phase));
    }
    if let Some(TaskEnd::Failed { reason }) = task.end() {
        resource = resource.with_error(reason.clone());
    }
    resource
}
