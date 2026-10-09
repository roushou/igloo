use axum::Json;
use axum::extract::State;
use igloo_api::worker::{WorkerAllocation, WorkerResource, WorkerUsage};
use igloo_core::sandbox::Sandbox;
use igloo_core::worker::Worker;
use igloo_core::{Entity, Labels, Resource};

use super::auth::Caller;
use super::{ApiError, ApiState};
use crate::app::AppError;

/// Lists the workers, ordered by id, with what their sandboxes hold of them.
#[utoipa::path(
    get,
    operation_id = "listWorkers",
    path = "/v1/workers",
    tag = "workers",
    responses((status = 200, body = Vec<WorkerResource>))
)]
pub(super) async fn list(
    State(state): State<ApiState>,
    _: Caller,
) -> Result<Json<Vec<WorkerResource>>, ApiError> {
    let mut workers = state.workers.all().await.map_err(AppError::from)?;
    workers.sort_by_key(Entity::id);
    let sandboxes = state
        .sandboxes
        .matching(&Labels::default())
        .await
        .map_err(AppError::from)?;
    let resources = workers
        .iter()
        .map(|worker| {
            let usage = state
                .usages
                .latest(worker.id())
                .map(|(usage, at)| WorkerUsage::new(&usage, at));
            WorkerResource::from(worker)
                .with_allocation(Allocation::of(worker, &sandboxes).into())
                .with_usage(usage)
        })
        .collect();
    Ok(Json(resources))
}

/// What the live sandboxes placed on one worker hold of it, by their limits.
#[derive(Default)]
struct Allocation {
    sandboxes: u32,
    millicpus: u64,
    memory_mib: u64,
}

impl Allocation {
    /// The allocation of `worker`: sandboxes assigned to it that have not ended.
    fn of(worker: &Worker, sandboxes: &[Sandbox]) -> Self {
        sandboxes
            .iter()
            .filter(|sandbox| {
                sandbox.status().worker() == Some(worker.id())
                    && !sandbox.status().phase().is_terminal()
            })
            .fold(Self::default(), |held, sandbox| {
                let limits = sandbox.spec().limits();
                Self {
                    sandboxes: held.sandboxes.saturating_add(1),
                    millicpus: held.millicpus + u64::from(limits.millicpus()),
                    memory_mib: held.memory_mib + u64::from(limits.memory_mib()),
                }
            })
    }
}

impl From<Allocation> for WorkerAllocation {
    fn from(held: Allocation) -> Self {
        Self::new(held.sandboxes, held.millicpus, held.memory_mib)
    }
}
