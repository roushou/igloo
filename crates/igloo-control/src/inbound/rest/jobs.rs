use std::collections::VecDeque;
use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::Stream;
use futures_util::stream;
use igloo_api::job::{JobPhase, JobResource};
use igloo_api::problem::Problem;
use igloo_core::job::JobId;
use igloo_core::process::OutputStream;
use tokio::sync::watch;

use crate::ports::{LogEntry, Sequence};

use super::auth::Caller;
use super::{ApiError, ApiState};
use crate::app::AppError;

/// Gets a job.
#[utoipa::path(
    get,
    operation_id = "getJob",
    path = "/v1/jobs/{id}",
    tag = "jobs",
    params(("id" = String, Path)),
    responses((status = 200, body = JobResource), (status = 404, body = Problem))
)]
pub(super) async fn get(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
) -> Result<Json<JobResource>, ApiError> {
    let id: JobId = id
        .parse()
        .map_err(|_| ApiError::not_found("job.not_found"))?;
    Ok(Json(load(&state, id).await?))
}

/// Streams a job's output as server-sent events: `stdout` and `stderr` events whose id resumes
/// the stream through `Last-Event-ID`, then one `end` event with the finished job.
#[utoipa::path(
    get,
    operation_id = "getJobLogs",
    path = "/v1/jobs/{id}/logs",
    tag = "jobs",
    params(
        ("id" = String, Path),
        ("Last-Event-ID" = Option<u64>, Header, description = "Resume after this event"),
    ),
    responses(
        (status = 200, content_type = "text/event-stream", body = String),
        (status = 404, body = Problem),
    )
)]
pub(super) async fn logs(
    State(state): State<ApiState>,
    _: Caller,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let job: JobId = id
        .parse()
        .map_err(|_| ApiError::not_found("job.not_found"))?;
    load(&state, job).await?;
    let after = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let follow = LogFollow {
        head: state.events.head(),
        state,
        job,
        after,
        pending: VecDeque::new(),
        done: false,
    };
    Ok(Sse::new(stream::unfold(follow, LogFollow::next)).keep_alive(KeepAlive::default()))
}

/// The state of one log stream.
struct LogFollow {
    state: ApiState,
    job: JobId,
    after: u64,
    pending: VecDeque<LogEntry>,
    head: watch::Receiver<Sequence>,
    done: bool,
}

impl LogFollow {
    /// How often new output is looked for while the job runs.
    const POLL: Duration = Duration::from_millis(250);
    const PAGE: usize = 256;

    async fn next(mut self) -> Option<(Result<Event, Infallible>, Self)> {
        loop {
            if self.done {
                return None;
            }
            if let Some(entry) = self.pending.pop_front() {
                self.after = entry.sequence;
                let name = match entry.stream {
                    OutputStream::Stdout => "stdout",
                    OutputStream::Stderr => "stderr",
                };
                let event = Event::default()
                    .id(entry.sequence.to_string())
                    .event(name)
                    .data(String::from_utf8_lossy(&entry.data));
                return Some((Ok(event), self));
            }
            match self.state.logs.read(self.job, self.after, Self::PAGE).await {
                Ok(entries) if !entries.is_empty() => {
                    self.pending.extend(entries);
                    continue;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(job = %self.job, %error, "reading logs failed");
                    self.done = true;
                    return Some((
                        Ok(Event::default().event("error").data("logs unavailable")),
                        self,
                    ));
                }
            }
            if let Ok(job) = load(&self.state, self.job).await
                && matches!(
                    job.phase,
                    JobPhase::Finished | JobPhase::Failed | JobPhase::Cancelled
                )
            {
                self.done = true;
                let data = serde_json::to_string(&job).unwrap_or_default();
                return Some((Ok(Event::default().event("end").data(data)), self));
            }
            tokio::select! {
                _ = self.head.changed() => {}
                () = tokio::time::sleep(Self::POLL) => {}
            }
        }
    }
}

pub(super) async fn load(state: &ApiState, id: JobId) -> Result<JobResource, ApiError> {
    let job = state
        .jobs
        .load(id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| ApiError::not_found("job.not_found"))?;
    Ok(JobResource::from(job.entity()))
}
