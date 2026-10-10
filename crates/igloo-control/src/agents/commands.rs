use std::sync::Arc;

use async_trait::async_trait;
use igloo_core::build::BuildId;
use igloo_core::change::ChangeId;
use igloo_core::job::{JobEnding, JobId};
use igloo_core::repo::{Repo, RepoId};
use igloo_core::sandbox::SandboxId;
use igloo_core::snapshot::SnapshotId;
use igloo_core::{Actor, Entity};

use super::ToolName;
use super::task::{Prepared, Task, TaskError, TaskId};
use crate::app::{AppError, Command, CommandHandler, RequestContext};
use crate::ports::{Clock, EntityStore, IdGenerator, IdGeneratorExt, StorageError, Versioned};

/// Creates a task on a repository.
pub struct CreateTask {
    /// The repository.
    pub repo: RepoId,
    /// What the agent is asked to do.
    pub goal: String,
    /// The tool; the repository's default when absent.
    pub tool: Option<ToolName>,
}

/// Records what preparing a task settled.
pub struct RecordTaskPrepared {
    /// The task.
    pub task: TaskId,
    /// What was settled.
    pub prepared: Prepared,
}

/// Records the build of a snapshot a task needs.
pub struct RecordTaskBuilding {
    /// The task.
    pub task: TaskId,
    /// The build.
    pub build: BuildId,
}

/// Records how a build a task waits on ended.
pub struct RecordTaskBuilt {
    /// The task.
    pub task: TaskId,
    /// The build.
    pub build: BuildId,
    /// Why the build failed, if it did.
    pub result: Result<(), String>,
}

/// Records the checkout a task's sandbox starts from.
pub struct RecordTaskReady {
    /// The task.
    pub task: TaskId,
    /// The snapshot.
    pub snapshot: SnapshotId,
}

/// Records a task's sandbox.
pub struct RecordTaskSandbox {
    /// The task.
    pub task: TaskId,
    /// The sandbox.
    pub sandbox: SandboxId,
}

/// Records a started turn.
pub struct RecordTurnStarted {
    /// The task.
    pub task: TaskId,
    /// The turn's job.
    pub job: JobId,
    /// What the tool was asked.
    pub prompt: String,
}

/// Records how a task's job ended.
pub struct RecordTurnEnded {
    /// The task.
    pub task: TaskId,
    /// The job.
    pub job: JobId,
    /// How it ended.
    pub ending: JobEnding,
    /// Whether the tool stopped on its account's usage limit.
    pub limited: bool,
}

/// Records the job collecting the last turn's commits.
pub struct RecordCommitsCollecting {
    /// The task.
    pub task: TaskId,
    /// The job.
    pub job: JobId,
}

/// Records the revision of the task's change the last turn's commits became.
pub struct RecordTaskRevised {
    /// The task.
    pub task: TaskId,
    /// The change.
    pub change: ChangeId,
    /// The revision's number.
    pub revision: u32,
}

/// Asks a task for another turn.
pub struct RequestTurn {
    /// The task.
    pub task: TaskId,
    /// What the tool is asked.
    pub prompt: String,
}

/// Ends a task as done: its change merged.
pub struct FinishTask {
    /// The task.
    pub task: TaskId,
}

/// Ends a task as failed.
pub struct FailTask {
    /// The task.
    pub task: TaskId,
    /// Why.
    pub reason: String,
}

/// Cancels a task.
pub struct CancelTask {
    /// The task.
    pub task: TaskId,
}

/// Records that a task's sandbox was stopped.
pub struct RecordTaskSandboxStopped {
    /// The task.
    pub task: TaskId,
}

/// Takes a task over for the person sending the command: their terminal in the task's sandbox
/// becomes writable and no turn starts until they hand it back. Refused for anyone but a
/// person.
pub struct TakeOverTask {
    /// The task.
    pub task: TaskId,
}

/// Hands a task back on behalf of the person who took it over: the sandbox is read for what
/// they changed and the task's next turn is asked about it.
pub struct HandBackTask {
    /// The task.
    pub task: TaskId,
}

/// Records the job reading what the person who took a task over changed.
pub struct RecordChangesReading {
    /// The task.
    pub task: TaskId,
    /// The job.
    pub job: JobId,
}

/// Records that a task was handed back, with the prompt naming what the person changed.
pub struct RecordHandedBack {
    /// The task.
    pub task: TaskId,
    /// What the tool is asked.
    pub prompt: String,
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
    CreateTask => TaskId, "task.create";
    RecordTaskPrepared => (), "task.record_prepared";
    RecordTaskBuilding => (), "task.record_building";
    RecordTaskBuilt => (), "task.record_built";
    RecordTaskReady => (), "task.record_ready";
    RecordTaskSandbox => (), "task.record_sandbox";
    RecordTurnStarted => (), "task.record_turn_started";
    RecordTurnEnded => (), "task.record_turn_ended";
    RecordCommitsCollecting => (), "task.record_commits_collecting";
    RecordTaskRevised => (), "task.record_revised";
    RequestTurn => (), "task.request_turn";
    FinishTask => (), "task.finish";
    FailTask => (), "task.fail";
    CancelTask => (), "task.cancel";
    RecordTaskSandboxStopped => (), "task.record_sandbox_stopped";
    TakeOverTask => (), "task.take_over";
    HandBackTask => (), "task.hand_back";
    RecordChangesReading => (), "task.record_changes_reading";
    RecordHandedBack => (), "task.record_handed_back";
}

/// Read access to tasks beyond loading one by id.
#[derive(Clone)]
pub struct TaskQueries {
    store: Arc<dyn EntityStore<Task>>,
}

impl TaskQueries {
    /// Queries over `store`.
    #[must_use]
    pub fn new(store: Arc<dyn EntityStore<Task>>) -> Self {
        Self { store }
    }

    /// Task `id`, if any.
    pub async fn get(&self, id: TaskId) -> Result<Option<Task>, StorageError> {
        Ok(self.store.load(id).await?.map(Versioned::into_inner))
    }

    /// The tasks of `repo`, oldest first.
    pub async fn of_repo(&self, repo: RepoId) -> Result<Vec<Task>, StorageError> {
        let mut tasks: Vec<Task> = self
            .store
            .all()
            .await?
            .into_iter()
            .filter(|task| task.repo() == repo)
            .collect();
        tasks.sort_by_key(Entity::id);
        Ok(tasks)
    }

    /// The task whose turn runs as `job`, collects its commits, or whose sandbox `job` reads for
    /// what a person changed.
    pub async fn with_job(&self, job: JobId) -> Result<Option<Task>, StorageError> {
        Ok(self.store.all().await?.into_iter().find(|task| {
            task.turns()
                .iter()
                .any(|turn| turn.job == job || turn.bundle == Some(job))
                || task.changes_job() == Some(job)
        }))
    }

    /// The task running in `sandbox`.
    pub async fn with_sandbox(&self, sandbox: SandboxId) -> Result<Option<Task>, StorageError> {
        Ok(self
            .store
            .all()
            .await?
            .into_iter()
            .find(|task| task.sandbox() == Some(sandbox)))
    }

    /// The task whose commits form `change`.
    pub async fn of_change(&self, change: ChangeId) -> Result<Option<Task>, StorageError> {
        Ok(self
            .store
            .all()
            .await?
            .into_iter()
            .find(|task| task.change() == Some(change)))
    }

    /// The tasks waiting on `build`.
    pub async fn of_build(&self, build: BuildId) -> Result<Vec<Task>, StorageError> {
        Ok(self
            .store
            .all()
            .await?
            .into_iter()
            .filter(|task| task.build() == Some(build))
            .collect())
    }
}

/// Creates a task after checking its repository exists.
pub(super) struct CreateTaskHandler {
    pub(super) store: Arc<dyn EntityStore<Task>>,
    pub(super) repos: Arc<dyn EntityStore<Repo>>,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) ids: Arc<dyn IdGenerator>,
}

#[async_trait]
impl CommandHandler<CreateTask> for CreateTaskHandler {
    async fn handle(
        &self,
        command: CreateTask,
        context: &RequestContext,
    ) -> Result<TaskId, AppError> {
        if self.repos.load(command.repo).await?.is_none() {
            return Err(AppError::not_found(Repo::NAME, &command.repo));
        }
        let id = self.ids.next::<Task>();
        let now = self.clock.now();
        let task = Task::new(id, command.repo, command.goal, command.tool, now)
            .map_err(|error| AppError::domain(&error))?;
        self.store
            .commit(&mut Versioned::new(task), &context.commit_meta(now))
            .await?;
        Ok(id)
    }
}

/// Takes tasks over and hands them back on behalf of the person sending the command.
#[derive(Clone)]
pub(super) struct TakeoverHandler {
    pub(super) store: Arc<dyn EntityStore<Task>>,
    pub(super) clock: Arc<dyn Clock>,
}

impl TakeoverHandler {
    /// How many times a change is attempted when commits lose races.
    const ATTEMPTS: usize = 3;

    /// Applies `change` to task `id` as the sender of the command, retrying from a fresh load
    /// when the commit loses a race.
    async fn apply(
        &self,
        id: TaskId,
        context: &RequestContext,
        change: impl Fn(&mut Task, Actor) -> Result<(), TaskError> + Send,
    ) -> Result<(), AppError> {
        let mut attempt = 1;
        loop {
            let mut task = self
                .store
                .load(id)
                .await?
                .ok_or_else(|| AppError::not_found(Task::NAME, &id))?;
            let now = self.clock.now();
            change(task.entity_mut(), context.actor).map_err(|error| AppError::domain(&error))?;
            match self
                .store
                .commit(&mut task, &context.commit_meta(now))
                .await
            {
                Ok(()) => return Ok(()),
                Err(error) => match AppError::from(error) {
                    AppError::Conflict if attempt < Self::ATTEMPTS => attempt += 1,
                    other => return Err(other),
                },
            }
        }
    }
}

#[async_trait]
impl CommandHandler<TakeOverTask> for TakeoverHandler {
    async fn handle(
        &self,
        command: TakeOverTask,
        context: &RequestContext,
    ) -> Result<(), AppError> {
        self.apply(command.task, context, Task::take_over).await
    }
}

#[async_trait]
impl CommandHandler<HandBackTask> for TakeoverHandler {
    async fn handle(
        &self,
        command: HandBackTask,
        context: &RequestContext,
    ) -> Result<(), AppError> {
        self.apply(command.task, context, Task::hand_back).await
    }
}
