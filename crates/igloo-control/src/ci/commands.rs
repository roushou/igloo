use std::sync::Arc;

use async_trait::async_trait;
use igloo_core::Entity;
use igloo_core::build::BuildId;
use igloo_core::change::ChangeId;
use igloo_core::job::JobId;
use igloo_core::repo::{CommitId, RepoId, WarmSnapshot};
use igloo_core::sandbox::SandboxId;
use igloo_core::snapshot::SnapshotId;

use super::run::{CheckOutcome, Planned, Run, RunId, WarmBuild};
use crate::app::{AppError, Command, CommandHandler, RequestContext};
use crate::ports::{Clock, EntityStore, IdGenerator, IdGeneratorExt, StorageError, Versioned};

/// Starts the run of a change's revision, unless it exists.
pub struct StartRun {
    /// The repository.
    pub repo: RepoId,
    /// The change.
    pub change: ChangeId,
    /// The revision's number.
    pub revision: u32,
    /// The revision's head.
    pub commit: CommitId,
}

/// Records the prepared pipeline.
pub struct RecordPrepared {
    /// The run.
    pub run: RunId,
    /// Sandbox settings and checks.
    pub plan: Planned,
    /// The snapshot to check, when no warm build is needed.
    pub snapshot: Option<SnapshotId>,
    /// The warm build to do first.
    pub warm: Option<WarmBuild>,
}

/// Records the build making a run's warm snapshot.
pub struct RecordWarmBuilding {
    /// The run.
    pub run: RunId,
    /// The build.
    pub build: BuildId,
}

/// Records how the build making a run's warm snapshot ended.
pub struct RecordWarmBuilt {
    /// The run.
    pub run: RunId,
    /// The build.
    pub build: BuildId,
    /// The warm snapshot, or why the build failed.
    pub warm: Result<WarmSnapshot, String>,
}

/// Records how one of a run's jobs ended.
pub struct RecordRunJob {
    /// The run.
    pub run: RunId,
    /// The job.
    pub job: JobId,
    /// How it ended.
    pub outcome: CheckOutcome,
}

/// Records the checkout over the warm snapshot.
pub struct RecordCheckedOut {
    /// The run.
    pub run: RunId,
    /// The snapshot to check.
    pub snapshot: SnapshotId,
}

/// Records the checks' sandbox and jobs.
pub struct RecordChecksStarted {
    /// The run.
    pub run: RunId,
    /// The sandbox.
    pub sandbox: SandboxId,
    /// One job per check, in check order.
    pub jobs: Vec<JobId>,
}

/// Ends a run as errored.
pub struct FailRun {
    /// The run.
    pub run: RunId,
    /// Why.
    pub reason: String,
}

/// Records that a run's sandbox was stopped.
pub struct RecordRunSandboxStopped {
    /// The run.
    pub run: RunId,
    /// The sandbox.
    pub sandbox: SandboxId,
}

macro_rules! commands {
    ($($command:ty => $output:ty, $name:literal;)*) => {
        $(impl Command for $command {
            type Output = $output;
            const NAME: &'static str = $name;
        })*
    };
}

commands! {
    StartRun => RunId, "run.start";
    RecordPrepared => (), "run.record_prepared";
    RecordWarmBuilding => (), "run.record_warm_building";
    RecordWarmBuilt => (), "run.record_warm_built";
    RecordRunJob => (), "run.record_job";
    RecordCheckedOut => (), "run.record_checked_out";
    RecordChecksStarted => (), "run.record_checks_started";
    FailRun => (), "run.fail";
    RecordRunSandboxStopped => (), "run.record_sandbox_stopped";
}

/// Read access to runs beyond loading one by id.
#[derive(Clone)]
pub struct RunQueries {
    store: Arc<dyn EntityStore<Run>>,
}

impl RunQueries {
    /// Queries over `store`.
    #[must_use]
    pub fn new(store: Arc<dyn EntityStore<Run>>) -> Self {
        Self { store }
    }

    /// Run `id`, if any.
    pub async fn get(&self, id: RunId) -> Result<Option<Run>, StorageError> {
        Ok(self.store.load(id).await?.map(Versioned::into_inner))
    }

    /// The runs of `change`, oldest first.
    pub async fn of_change(&self, change: ChangeId) -> Result<Vec<Run>, StorageError> {
        let mut runs: Vec<Run> = self
            .store
            .all()
            .await?
            .into_iter()
            .filter(|run| run.change() == change)
            .collect();
        runs.sort_by_key(Entity::id);
        Ok(runs)
    }

    /// The run waiting on `job` as one of its checks.
    pub async fn with_job(&self, job: JobId) -> Result<Option<Run>, StorageError> {
        Ok(self
            .store
            .all()
            .await?
            .into_iter()
            .find(|run| run.checks().iter().any(|check| check.job == Some(job))))
    }

    /// The runs whose warm snapshot `build` makes.
    pub async fn of_build(&self, build: BuildId) -> Result<Vec<Run>, StorageError> {
        Ok(self
            .store
            .all()
            .await?
            .into_iter()
            .filter(|run| run.warm_build() == Some(build))
            .collect())
    }
}

/// Starts a run unless the revision already has one.
pub(super) struct StartRunHandler {
    pub(super) queries: RunQueries,
    pub(super) store: Arc<dyn EntityStore<Run>>,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) ids: Arc<dyn IdGenerator>,
}

#[async_trait]
impl CommandHandler<StartRun> for StartRunHandler {
    async fn handle(&self, command: StartRun, context: &RequestContext) -> Result<RunId, AppError> {
        let existing = self
            .queries
            .of_change(command.change)
            .await?
            .into_iter()
            .find(|run| run.revision() == command.revision);
        if let Some(run) = existing {
            return Ok(run.id());
        }
        let id = self.ids.next::<Run>();
        let now = self.clock.now();
        let run = Run::new(
            id,
            command.repo,
            (command.change, command.revision),
            command.commit,
            now,
        );
        self.store
            .commit(&mut Versioned::new(run), &context.commit_meta(now))
            .await?;
        Ok(id)
    }
}
