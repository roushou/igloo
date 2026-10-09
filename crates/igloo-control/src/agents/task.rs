use igloo_core::build::BuildId;
use igloo_core::change::{ChangeId, ChangeRequest};
use igloo_core::job::{JobEnding, JobId};
use igloo_core::repo::{CommitId, RepoId};
use igloo_core::sandbox::SandboxId;
use igloo_core::snapshot::SnapshotId;
use igloo_core::{
    Entity, ErrorCode, Event, Generation, Id, Labels, Plan, Prefixed, Resource, Timestamp,
};
use jiff::SignedDuration;
use serde::{Deserialize, Serialize};

use super::{ToolName, ToolSpec};
use crate::ci::{SandboxSettings, Warm};

/// Identifies a task (`task_...`).
pub type TaskId = Id<Task>;

/// One agent on one goal in one sandbox: the tool works in turns, each a job in the task's
/// sandbox; the first turn gets the goal, later ones a reviewer's request.
///
/// Invariant: a task ends once, done, failed or cancelled; at most one turn runs at a time and
/// at most the tool's `max_turns` run; the sandbox is stopped once the task ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    id: TaskId,
    repo: RepoId,
    goal: String,
    tool: Option<ToolName>,
    prepared: Option<Prepared>,
    setup: Setup,
    sandbox: Option<SandboxId>,
    stopped: bool,
    released: bool,
    turns: Vec<Turn>,
    limited: u32,
    retry_at: Option<Timestamp>,
    requested: Option<String>,
    change: Option<ChangeId>,
    end: Option<TaskEnd>,
    created_at: Timestamp,
    events: Vec<TaskEvent>,
}

/// What preparing a task settled, from the repository's default branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prepared {
    /// The commit the task starts from: the default branch's head.
    pub commit: CommitId,
    /// The tool's name.
    pub tool: ToolName,
    /// The tool.
    pub spec: ToolSpec,
    /// The sandbox settings, from the pipeline.
    pub settings: SandboxSettings,
    /// The snapshot the checkout goes over.
    pub base: Option<SnapshotId>,
    /// How the warm snapshot is built, if the repository has one.
    pub warm: Option<Warm>,
}

/// One turn of the tool: its job, then the job collecting its commits, then the revision of
/// the task's change they became.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    /// The job running it.
    pub job: JobId,
    /// What the tool was asked.
    pub prompt: String,
    /// How it ended, once it has.
    pub ending: Option<JobEnding>,
    /// The job collecting its commits, once started.
    pub bundle: Option<JobId>,
    /// Whether that job succeeded, once it ended.
    pub bundled: bool,
    /// How many collecting jobs failed; a failed one is retried until
    /// [`Task::COLLECT_ATTEMPTS`] have failed.
    #[serde(default)]
    pub collect_failures: u32,
    /// The revision of the task's change its commits became.
    pub revision: Option<u32>,
}

/// How a task ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "end", rename_all = "snake_case")]
pub enum TaskEnd {
    /// Its change merged.
    Done,
    /// It could not continue.
    Failed {
        /// Why.
        reason: String,
    },
    /// Someone cancelled it.
    Cancelled,
}

/// Where a task is, as reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskPhase {
    /// Reading its settings and building its snapshot.
    Preparing,
    /// A turn runs, or is about to.
    Working,
    /// Its last turn ended; waiting for a review.
    AwaitingReview,
    /// Ended; see [`Task::end`].
    Ended,
}

/// Where setting up the task's snapshot is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setup {
    /// The next step is to be decided.
    Pending,
    /// A snapshot it needs is being built.
    Building(BuildId),
    /// The checkout the sandbox starts from.
    Ready(SnapshotId),
}

/// What the agent controller may be asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskAction {
    /// Read the settings once, then find the task's snapshot or start the build it needs.
    Prepare,
    /// Start the sandbox from the snapshot.
    StartSandbox(SnapshotId),
    /// Start a turn with the prompt.
    StartTurn(String),
    /// Collect the last turn's commits.
    CollectCommits,
    /// Publish the commits the job collected as the next revision of the task's change.
    Publish(JobId),
    /// Stop the sandbox.
    StopSandbox(SandboxId),
}

/// Facts about a task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskEvent {
    /// A task was created.
    Created {
        /// Its id.
        id: TaskId,
        /// The repository.
        repo: RepoId,
        /// What the agent is asked to do.
        goal: String,
        /// The tool asked for; the repository's default when absent.
        tool: Option<ToolName>,
        /// When.
        at: Timestamp,
    },
    /// The settings were read.
    Prepared {
        /// What was settled.
        prepared: Box<Prepared>,
    },
    /// A snapshot the task needs is being built.
    Building {
        /// The build.
        build: BuildId,
    },
    /// That build completed; the next step is decided again.
    Built {
        /// The build.
        build: BuildId,
    },
    /// The checkout the sandbox starts from is ready.
    Ready {
        /// The snapshot.
        snapshot: SnapshotId,
    },
    /// The sandbox was created.
    SandboxStarted {
        /// The sandbox.
        sandbox: SandboxId,
    },
    /// A reviewer asked for another turn.
    TurnRequested {
        /// What the tool is asked.
        prompt: String,
    },
    /// A turn started.
    TurnStarted {
        /// Its job.
        job: JobId,
        /// What the tool was asked.
        prompt: String,
    },
    /// A turn ended.
    TurnEnded {
        /// Its job.
        job: JobId,
        /// How.
        ending: JobEnding,
    },
    /// The last turn's commits are being collected.
    CommitsCollecting {
        /// The collecting job.
        job: JobId,
    },
    /// The collecting job ended.
    CommitsCollected {
        /// The collecting job.
        job: JobId,
        /// How.
        ending: JobEnding,
    },
    /// The last turn's commits became a revision of the task's change.
    Revised {
        /// The change.
        change: ChangeId,
        /// The revision's number.
        revision: u32,
    },
    /// The task ended. Final.
    Ended {
        /// How.
        end: TaskEnd,
    },
    /// The sandbox was stopped.
    SandboxStopped,
    /// The sandbox a failed collection kept may be stopped.
    SandboxReleased,
    /// A turn stopped on its tool's usage limit. It is dropped and its prompt asked again at
    /// `retry_at`.
    TurnLimited {
        /// The turn's job.
        job: JobId,
        /// How it ended.
        ending: JobEnding,
        /// When the prompt is asked again.
        retry_at: Timestamp,
    },
}

/// Why a task change is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TaskError {
    /// The change does not apply to the task's state.
    #[error("the task is not at that step")]
    OutOfOrder,
    /// The goal is empty.
    #[error("the goal is empty")]
    EmptyGoal,
}

impl ErrorCode for TaskError {
    fn code(&self) -> &'static str {
        match self {
            Self::OutOfOrder => "task.out_of_order",
            Self::EmptyGoal => "task.empty_goal",
        }
    }
}

impl Task {
    /// How many collecting jobs of one turn may fail before the task fails.
    pub const COLLECT_ATTEMPTS: u32 = 3;
    /// How long a turn stopped by its tool's usage limit waits before it is asked again.
    pub const LIMIT_RETRY: SignedDuration = SignedDuration::from_mins(30);
    /// How many times a task's turns may stop on the usage limit before the task fails: six
    /// hours of retries, longer than a usage window.
    pub const LIMIT_ATTEMPTS: u32 = 12;
    /// The most characters a change title taken from a goal has.
    pub const TITLE_CHARS: usize = 72;

    /// A task on `repo` working toward `goal` with `tool`, or the repository's default tool.
    pub fn new(
        id: TaskId,
        repo: RepoId,
        goal: String,
        tool: Option<ToolName>,
        now: Timestamp,
    ) -> Result<Self, TaskError> {
        if goal.trim().is_empty() {
            return Err(TaskError::EmptyGoal);
        }
        let mut task = Self::initial(id, repo, goal.clone(), tool.clone(), now);
        task.events.push(TaskEvent::Created {
            id,
            repo,
            goal,
            tool,
            at: now,
        });
        Ok(task)
    }

    /// Records the settled settings; once.
    pub fn prepared(&mut self, prepared: Prepared) -> Result<(), TaskError> {
        if self.prepared.is_some() || self.end.is_some() {
            return Err(TaskError::OutOfOrder);
        }
        self.record(TaskEvent::Prepared {
            prepared: Box::new(prepared),
        });
        Ok(())
    }

    /// Records the build of a snapshot the task needs.
    pub fn building(&mut self, build: BuildId) -> Result<(), TaskError> {
        self.deciding()?;
        self.record(TaskEvent::Building { build });
        Ok(())
    }

    /// Records how a build ended: the task decides its next step, or fails with the build.
    /// Other builds and repeated reports are ignored.
    pub fn built(&mut self, build: BuildId, result: Result<(), String>) {
        if self.setup != Setup::Building(build) || self.end.is_some() {
            return;
        }
        match result {
            Ok(()) => self.record(TaskEvent::Built { build }),
            Err(reason) => self.end_with(TaskEnd::Failed {
                reason: format!("the snapshot: {reason}"),
            }),
        }
    }

    /// Records the checkout the sandbox starts from.
    pub fn ready(&mut self, snapshot: SnapshotId) -> Result<(), TaskError> {
        self.deciding()?;
        self.record(TaskEvent::Ready { snapshot });
        Ok(())
    }

    /// Records the task's sandbox.
    pub fn sandbox_started(&mut self, sandbox: SandboxId) -> Result<(), TaskError> {
        if !matches!(self.setup, Setup::Ready(_)) || self.sandbox.is_some() || self.end.is_some() {
            return Err(TaskError::OutOfOrder);
        }
        self.record(TaskEvent::SandboxStarted { sandbox });
        Ok(())
    }

    /// Asks for another turn with `prompt`, started once the last turn's commits are a
    /// revision; a prompt asked while one waits is added to it. A task that already took its
    /// tool's `max_turns` fails instead; an ended task ignores the request.
    pub fn request_turn(&mut self, prompt: String) {
        if self.end.is_some() {
            return;
        }
        let max = self
            .prepared
            .as_ref()
            .map_or(u32::MAX, |prepared| prepared.spec.max_turns);
        let turns = u32::try_from(self.turns.len()).unwrap_or(u32::MAX);
        if self.requested.is_none() && turns >= max {
            self.end_with(TaskEnd::Failed {
                reason: format!("the task took its {max} turns"),
            });
        } else {
            self.record(TaskEvent::TurnRequested { prompt });
        }
    }

    /// Ends the task as done: its change merged.
    pub fn finish(&mut self) {
        if self.end.is_none() {
            self.end_with(TaskEnd::Done);
        }
    }

    /// The prompt of the turn answering `request`.
    #[must_use]
    pub fn review_prompt(request: &ChangeRequest) -> String {
        let comments: Vec<String> = request
            .comments
            .iter()
            .map(|comment| match (&comment.path, comment.line) {
                (Some(path), Some(line)) => format!("- {path}:{line}: {}", comment.body),
                (Some(path), None) => format!("- {path}: {}", comment.body),
                _ => format!("- {}", comment.body),
            })
            .collect();
        format!(
            "A reviewer asked for changes to revision {}:\n\n{}\n\nAddress every comment.",
            request.revision,
            comments.join("\n")
        )
    }

    /// Records the started turn.
    pub fn turn_started(&mut self, job: JobId, prompt: String) -> Result<(), TaskError> {
        if self.next_prompt().as_deref() != Some(prompt.as_str()) {
            return Err(TaskError::OutOfOrder);
        }
        self.record(TaskEvent::TurnStarted { job, prompt });
        Ok(())
    }

    /// Records the job collecting the last turn's commits.
    pub fn collecting(&mut self, job: JobId) -> Result<(), TaskError> {
        let ready = self.turns.last().is_some_and(|turn| {
            turn.ending.is_some_and(|ending| ending.succeeded()) && turn.bundle.is_none()
        });
        if !ready || self.end.is_some() {
            return Err(TaskError::OutOfOrder);
        }
        self.record(TaskEvent::CommitsCollecting { job });
        Ok(())
    }

    /// Records how a job ended; only the last turn's job and its collecting job count, once
    /// each. A failed turn fails the task. A failed collection is retried; once
    /// [`Task::COLLECT_ATTEMPTS`] have failed, the task fails and keeps its sandbox, so the
    /// turn's commits stay recoverable, until it is cancelled.
    pub fn job_ended(&mut self, job: JobId, ending: JobEnding) {
        let Some(turn) = self.turns.last() else {
            return;
        };
        let number = self.turns.len();
        if self.end.is_some() {
            return;
        }
        if turn.job == job && turn.ending.is_none() {
            self.record(TaskEvent::TurnEnded { job, ending });
            if !ending.succeeded() {
                self.end_with(TaskEnd::Failed {
                    reason: format!("turn {number} {ending}"),
                });
            }
        } else if turn.bundle == Some(job) && !turn.bundled {
            self.record(TaskEvent::CommitsCollected { job, ending });
            let failures = self.turns.last().map_or(0, |turn| turn.collect_failures);
            if failures >= Self::COLLECT_ATTEMPTS {
                self.end_with(TaskEnd::Failed {
                    reason: format!(
                        "collecting the commits of turn {number} {ending}, {failures} times; \
                         the sandbox is kept for recovery until the task is cancelled"
                    ),
                });
            }
        }
    }

    /// Records the revision of `change` the last turn's commits became. A task's revisions all
    /// belong to its one change.
    pub fn revised(&mut self, change: ChangeId, revision: u32) -> Result<(), TaskError> {
        let ready = self
            .turns
            .last()
            .is_some_and(|turn| turn.bundled && turn.revision.is_none());
        if !ready || self.end.is_some() || self.change.is_some_and(|own| own != change) {
            return Err(TaskError::OutOfOrder);
        }
        self.record(TaskEvent::Revised { change, revision });
        Ok(())
    }

    /// Records that turn `job` stopped on its tool's usage limit at `now`: the turn is dropped
    /// and its prompt asked again after [`Task::LIMIT_RETRY`], and the sandbox keeps the
    /// tool's work. After [`Task::LIMIT_ATTEMPTS`] such stops the task fails. Other jobs, and a
    /// turn already ended, change nothing.
    pub fn turn_limited(&mut self, job: JobId, ending: JobEnding, now: Timestamp) {
        let current = self
            .turns
            .last()
            .is_some_and(|turn| turn.job == job && turn.ending.is_none());
        if self.end.is_some() || !current {
            return;
        }
        if self.limited + 1 >= Self::LIMIT_ATTEMPTS {
            self.record(TaskEvent::TurnEnded { job, ending });
            self.end_with(TaskEnd::Failed {
                reason: format!(
                    "the tool's usage limit stopped its turns {} times",
                    Self::LIMIT_ATTEMPTS
                ),
            });
            return;
        }
        self.record(TaskEvent::TurnLimited {
            job,
            ending,
            retry_at: now.saturating_add(Self::LIMIT_RETRY),
        });
    }

    /// The title of the task's change: the goal's first line, cut at a word to at most
    /// [`Task::TITLE_CHARS`] characters, an ellipsis marking the cut.
    #[must_use]
    pub fn title(&self) -> String {
        let line = self.goal.lines().next().unwrap_or_default().trim();
        if line.chars().count() <= Self::TITLE_CHARS {
            return line.to_owned();
        }
        let head: String = line.chars().take(Self::TITLE_CHARS - 1).collect();
        let cut = head
            .rfind(char::is_whitespace)
            .map_or(head.as_str(), |end| head[..end].trim_end());
        format!("{cut}…")
    }

    /// Cancels the task, unless it ended. Cancelling a task that failed collecting its
    /// commits releases the sandbox it kept.
    pub fn cancel(&mut self) {
        if self.end.is_none() {
            self.end_with(TaskEnd::Cancelled);
        } else if self.keeps_sandbox() {
            self.record(TaskEvent::SandboxReleased);
        }
    }

    /// Whether the task ended keeping its sandbox: its last collection failed every attempt
    /// and nobody released the sandbox yet.
    #[must_use]
    pub fn keeps_sandbox(&self) -> bool {
        matches!(self.end, Some(TaskEnd::Failed { .. }))
            && !self.released
            && self
                .turns
                .last()
                .is_some_and(|turn| turn.collect_failures >= Self::COLLECT_ATTEMPTS)
    }

    /// Fails the task, unless it ended.
    pub fn fail(&mut self, reason: String) {
        if self.end.is_none() {
            self.end_with(TaskEnd::Failed { reason });
        }
    }

    /// Records that the sandbox was stopped.
    pub fn sandbox_stopped(&mut self) {
        if self.sandbox.is_some() && !self.stopped {
            self.record(TaskEvent::SandboxStopped);
        }
    }

    /// The repository.
    #[must_use]
    pub const fn repo(&self) -> RepoId {
        self.repo
    }

    /// What the agent is asked to do.
    #[must_use]
    pub fn goal(&self) -> &str {
        &self.goal
    }

    /// The tool asked for; the repository's default when absent.
    #[must_use]
    pub const fn tool(&self) -> Option<&ToolName> {
        self.tool.as_ref()
    }

    /// What preparing settled, once prepared.
    #[must_use]
    pub const fn settings(&self) -> Option<&Prepared> {
        self.prepared.as_ref()
    }

    /// The build the task waits on, if any.
    #[must_use]
    pub const fn build(&self) -> Option<BuildId> {
        match self.setup {
            Setup::Building(build) => Some(build),
            _ => None,
        }
    }

    /// The sandbox, once started.
    #[must_use]
    pub const fn sandbox(&self) -> Option<SandboxId> {
        self.sandbox
    }

    /// The turns, oldest first.
    #[must_use]
    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    /// The change the task's commits form, once it has one.
    #[must_use]
    pub const fn change(&self) -> Option<ChangeId> {
        self.change
    }

    /// How it ended, once it has.
    #[must_use]
    pub const fn end(&self) -> Option<&TaskEnd> {
        self.end.as_ref()
    }

    /// When it was created.
    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    /// Where it is.
    #[must_use]
    pub fn phase(&self) -> TaskPhase {
        if self.end.is_some() {
            TaskPhase::Ended
        } else if self.sandbox.is_none() {
            TaskPhase::Preparing
        } else if self.next_prompt().is_none()
            && self
                .turns
                .last()
                .is_some_and(|turn| turn.revision.is_some())
        {
            TaskPhase::AwaitingReview
        } else {
            TaskPhase::Working
        }
    }

    /// The prompt of the turn to start next, if one is due: the goal first, then requests.
    fn next_prompt(&self) -> Option<String> {
        if self.sandbox.is_none() || self.end.is_some() {
            return None;
        }
        if self.turns.is_empty() {
            return Some(self.goal.clone());
        }
        let last_revised = self
            .turns
            .last()
            .is_some_and(|turn| turn.revision.is_some());
        self.requested.clone().filter(|_| last_revised)
    }

    fn deciding(&self) -> Result<(), TaskError> {
        if self.setup != Setup::Pending || self.prepared.is_none() || self.end.is_some() {
            return Err(TaskError::OutOfOrder);
        }
        Ok(())
    }

    fn end_with(&mut self, end: TaskEnd) {
        self.record(TaskEvent::Ended { end });
    }

    fn initial(
        id: TaskId,
        repo: RepoId,
        goal: String,
        tool: Option<ToolName>,
        at: Timestamp,
    ) -> Self {
        Self {
            id,
            repo,
            goal,
            tool,
            prepared: None,
            setup: Setup::Pending,
            sandbox: None,
            stopped: false,
            released: false,
            turns: Vec::new(),
            limited: 0,
            retry_at: None,
            requested: None,
            change: None,
            end: None,
            created_at: at,
            events: Vec::new(),
        }
    }

    fn record(&mut self, event: TaskEvent) {
        self.apply(&event);
        self.events.push(event);
    }
}

impl Prefixed for Task {
    const PREFIX: &'static str = "task";
}

impl Event for TaskEvent {
    const SCHEMA_VERSION: u16 = 1;

    fn kind(&self) -> &'static str {
        match self {
            Self::Created { .. } => "igloo.task.created",
            Self::Prepared { .. } => "igloo.task.prepared",
            Self::Building { .. } => "igloo.task.building",
            Self::Built { .. } => "igloo.task.built",
            Self::Ready { .. } => "igloo.task.ready",
            Self::SandboxStarted { .. } => "igloo.task.sandbox_started",
            Self::TurnRequested { .. } => "igloo.task.turn_requested",
            Self::TurnStarted { .. } => "igloo.task.turn_started",
            Self::TurnEnded { .. } => "igloo.task.turn_ended",
            Self::CommitsCollecting { .. } => "igloo.task.commits_collecting",
            Self::CommitsCollected { .. } => "igloo.task.commits_collected",
            Self::Revised { .. } => "igloo.task.revised",
            Self::Ended { .. } => "igloo.task.ended",
            Self::SandboxStopped => "igloo.task.sandbox_stopped",
            Self::SandboxReleased => "igloo.task.sandbox_released",
            Self::TurnLimited { .. } => "igloo.task.turn_limited",
        }
    }
}

impl Entity for Task {
    const NAME: &'static str = "task";

    type Event = TaskEvent;

    fn id(&self) -> TaskId {
        self.id
    }

    fn from_created(event: &TaskEvent) -> Option<Self> {
        let TaskEvent::Created {
            id,
            repo,
            goal,
            tool,
            at,
        } = event
        else {
            return None;
        };
        Some(Self::initial(*id, *repo, goal.clone(), tool.clone(), *at))
    }

    fn apply(&mut self, event: &TaskEvent) {
        match event {
            TaskEvent::Created { .. } => {}
            TaskEvent::Prepared { prepared } => self.prepared = Some(prepared.as_ref().clone()),
            TaskEvent::Building { build } => self.setup = Setup::Building(*build),
            TaskEvent::Built { .. } => self.setup = Setup::Pending,
            TaskEvent::Ready { snapshot } => self.setup = Setup::Ready(*snapshot),
            TaskEvent::SandboxStarted { sandbox } => self.sandbox = Some(*sandbox),
            TaskEvent::TurnRequested { prompt } => {
                self.requested = Some(match self.requested.take() {
                    Some(pending) => format!("{pending}\n\n{prompt}"),
                    None => prompt.clone(),
                });
            }
            TaskEvent::TurnStarted { job, prompt } => {
                self.requested = None;
                self.turns.push(Turn {
                    job: *job,
                    prompt: prompt.clone(),
                    ending: None,
                    bundle: None,
                    bundled: false,
                    collect_failures: 0,
                    revision: None,
                });
            }
            TaskEvent::TurnEnded { job, ending } => {
                if let Some(turn) = self.turns.iter_mut().find(|turn| turn.job == *job) {
                    turn.ending = Some(*ending);
                }
            }
            TaskEvent::CommitsCollecting { job } => {
                if let Some(turn) = self.turns.last_mut() {
                    turn.bundle = Some(*job);
                }
            }
            TaskEvent::CommitsCollected { ending, .. } => {
                if let Some(turn) = self.turns.last_mut() {
                    if ending.succeeded() {
                        turn.bundled = true;
                    } else {
                        turn.bundle = None;
                        turn.collect_failures += 1;
                    }
                }
            }
            TaskEvent::Revised { change, revision } => {
                self.change = Some(*change);
                if let Some(turn) = self.turns.last_mut() {
                    turn.revision = Some(*revision);
                }
            }
            TaskEvent::Ended { end } => self.end = Some(end.clone()),
            TaskEvent::SandboxStopped => self.stopped = true,
            TaskEvent::SandboxReleased => self.released = true,
            TaskEvent::TurnLimited { job, retry_at, .. } => {
                if self.turns.last().is_some_and(|turn| turn.job == *job)
                    && let Some(turn) = self.turns.pop()
                {
                    // A turn after the first answers what was requested; ask it again, with
                    // whatever was requested since.
                    if !self.turns.is_empty() {
                        self.requested = Some(match self.requested.take() {
                            Some(pending) => format!("{}\n\n{pending}", turn.prompt),
                            None => turn.prompt,
                        });
                    }
                }
                self.limited += 1;
                self.retry_at = Some(*retry_at);
            }
        }
    }

    fn take_events(&mut self) -> Vec<TaskEvent> {
        std::mem::take(&mut self.events)
    }
}

/// Tasks carry no labels.
static NO_LABELS: Labels = Labels::EMPTY;

impl Resource for Task {
    type Spec = String;
    type Status = Option<TaskEnd>;
    type Action = TaskAction;

    fn spec(&self) -> &String {
        &self.goal
    }

    fn status(&self) -> &Option<TaskEnd> {
        &self.end
    }

    fn labels(&self) -> &Labels {
        &NO_LABELS
    }

    /// A task's goal never changes.
    fn generation(&self) -> Generation {
        Generation::INITIAL
    }

    /// One step at a time; steps waiting on a build or a turn resume when its outcome is
    /// recorded. An ended task stops its sandbox, unless it keeps it for recovery.
    fn plan(&self, now: Timestamp) -> Plan<TaskAction> {
        if self.keeps_sandbox() {
            return Plan::Converged;
        }
        if self.end.is_some() {
            return match self.sandbox {
                Some(sandbox) if !self.stopped => Plan::Act(vec![TaskAction::StopSandbox(sandbox)]),
                _ => Plan::Converged,
            };
        }
        match (self.setup, self.sandbox) {
            (Setup::Pending, _) => Plan::Act(vec![TaskAction::Prepare]),
            (Setup::Building(_), _) => Plan::Converged,
            (Setup::Ready(snapshot), None) => Plan::Act(vec![TaskAction::StartSandbox(snapshot)]),
            (Setup::Ready(_), Some(_)) => {
                if let Some(prompt) = self.next_prompt() {
                    if let Some(retry_at) = self.retry_at
                        && retry_at > now
                    {
                        return Plan::Recheck {
                            after: retry_at.duration_since(now),
                        };
                    }
                    return Plan::Act(vec![TaskAction::StartTurn(prompt)]);
                }
                match self.turns.last() {
                    Some(turn)
                        if turn.ending.is_some_and(|ending| ending.succeeded())
                            && turn.bundle.is_none() =>
                    {
                        Plan::Act(vec![TaskAction::CollectCommits])
                    }
                    Some(Turn {
                        bundle: Some(job),
                        bundled: true,
                        revision: None,
                        ..
                    }) => Plan::Act(vec![TaskAction::Publish(*job)]),
                    _ => Plan::Converged,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use igloo_core::Digest;
    use igloo_core::job::JobFailure;
    use igloo_core::process::EnvVars;
    use igloo_core::sandbox::{Isolation, NetworkPolicy, ResourceLimits};
    use igloo_core::testing::Scenario;
    use uuid::Uuid;

    use super::*;
    use crate::agents::HarnessKind;

    type S = Scenario<Task>;

    fn id<T: Prefixed>(n: u128) -> Id<T> {
        Id::from_uuid(Uuid::from_u128(n))
    }

    fn snapshot(n: u8) -> SnapshotId {
        SnapshotId::from(Digest::from_blake3([n; 32]))
    }

    fn created() -> TaskEvent {
        TaskEvent::Created {
            id: S::ID,
            repo: id(1),
            goal: "Fix the bug".to_owned(),
            tool: None,
            at: S::NOW,
        }
    }

    fn prepared(max_turns: u32) -> Prepared {
        Prepared {
            commit: "a".repeat(40).parse().expect("commit"),
            tool: "agent".parse().expect("tool"),
            spec: ToolSpec {
                harness: HarnessKind::Command,
                install: Some("install".to_owned()),
                command: Some("./agent".to_owned()),
                secrets: BTreeSet::new(),
                timeout_seconds: 60,
                max_turns,
            },
            settings: SandboxSettings {
                isolation: Isolation::Any,
                limits: ResourceLimits::default(),
                network: NetworkPolicy::DenyAll,
                env: EnvVars::default(),
                secrets: BTreeSet::new(),
            },
            base: None,
            warm: None,
        }
    }

    fn working(max_turns: u32) -> Vec<TaskEvent> {
        vec![
            created(),
            TaskEvent::Prepared {
                prepared: Box::new(prepared(max_turns)),
            },
            TaskEvent::Ready {
                snapshot: snapshot(3),
            },
            TaskEvent::SandboxStarted { sandbox: id(10) },
            TaskEvent::TurnStarted {
                job: id(20),
                prompt: "Fix the bug".to_owned(),
            },
        ]
    }

    fn exited(code: i32) -> JobEnding {
        JobEnding::Exited { code }
    }

    #[test]
    fn a_task_needs_a_goal() {
        assert_eq!(
            Task::new(id(1), id(2), " ".to_owned(), None, S::NOW),
            Err(TaskError::EmptyGoal)
        );
    }

    #[test]
    fn a_task_prepares_builds_until_ready_then_starts_its_sandbox_and_first_turn() {
        S::given([created()])
            .plan()
            .then_actions([TaskAction::Prepare]);
        let deciding = S::given([created()])
            .try_when(|task, _| task.prepared(prepared(10)))
            .then([TaskEvent::Prepared {
                prepared: Box::new(prepared(10)),
            }]);
        deciding.plan().then_actions([TaskAction::Prepare]);
        let building = deciding
            .try_when(|task, _| task.building(id(40)))
            .then([TaskEvent::Building { build: id(40) }]);
        assert_eq!(building.state().build(), Some(id(40)));
        building.plan().then_converged();
        let built = building
            .when(|task, _| task.built(id(41), Ok(())))
            .then_no_events()
            .when(|task, _| task.built(id(40), Ok(())))
            .then([TaskEvent::Built { build: id(40) }]);
        built.plan().then_actions([TaskAction::Prepare]);
        let ready = built
            .try_when(|task, _| task.ready(snapshot(3)))
            .then([TaskEvent::Ready {
                snapshot: snapshot(3),
            }]);
        assert_eq!(ready.state().phase(), TaskPhase::Preparing);
        ready
            .plan()
            .then_actions([TaskAction::StartSandbox(snapshot(3))]);
        let started = ready
            .try_when(|task, _| task.sandbox_started(id(10)))
            .then([TaskEvent::SandboxStarted { sandbox: id(10) }]);
        assert_eq!(started.state().phase(), TaskPhase::Working);
        started
            .plan()
            .then_actions([TaskAction::StartTurn("Fix the bug".to_owned())]);
        started
            .try_when(|task, _| task.turn_started(id(20), "Fix the bug".to_owned()))
            .then([TaskEvent::TurnStarted {
                job: id(20),
                prompt: "Fix the bug".to_owned(),
            }])
            .plan()
            .then_converged();
    }

    #[test]
    fn a_failed_build_fails_the_task() {
        S::given([
            created(),
            TaskEvent::Prepared {
                prepared: Box::new(prepared(10)),
            },
            TaskEvent::Building { build: id(40) },
        ])
        .when(|task, _| task.built(id(40), Err("the build exited with 1".to_owned())))
        .then([TaskEvent::Ended {
            end: TaskEnd::Failed {
                reason: "the snapshot: the build exited with 1".to_owned(),
            },
        }])
        .plan()
        .then_converged();
    }

    /// `working` with its first turn ended, collected and revised.
    fn revised(max_turns: u32) -> Vec<TaskEvent> {
        let mut events = working(max_turns);
        events.extend([
            TaskEvent::TurnEnded {
                job: id(20),
                ending: exited(0),
            },
            TaskEvent::CommitsCollecting { job: id(30) },
            TaskEvent::CommitsCollected {
                job: id(30),
                ending: exited(0),
            },
            TaskEvent::Revised {
                change: id(50),
                revision: 1,
            },
        ]);
        events
    }

    #[test]
    fn an_ended_turn_has_its_commits_collected_and_published_then_awaits_review() {
        let ended = S::given(working(10))
            .when(|task, _| task.job_ended(id(99), exited(0)))
            .then_no_events()
            .when(|task, _| task.job_ended(id(20), exited(0)))
            .then([TaskEvent::TurnEnded {
                job: id(20),
                ending: exited(0),
            }]);
        assert_eq!(ended.state().phase(), TaskPhase::Working);
        ended.plan().then_actions([TaskAction::CollectCommits]);
        let collecting = ended
            .try_when(|task, _| task.collecting(id(30)))
            .then([TaskEvent::CommitsCollecting { job: id(30) }]);
        collecting.plan().then_converged();
        let collected = collecting
            .when(|task, _| task.job_ended(id(30), exited(0)))
            .then([TaskEvent::CommitsCollected {
                job: id(30),
                ending: exited(0),
            }]);
        collected.plan().then_actions([TaskAction::Publish(id(30))]);
        let awaiting =
            collected
                .try_when(|task, _| task.revised(id(50), 1))
                .then([TaskEvent::Revised {
                    change: id(50),
                    revision: 1,
                }]);
        assert_eq!(awaiting.state().phase(), TaskPhase::AwaitingReview);
        assert_eq!(awaiting.state().change(), Some(id(50)));
        awaiting.plan().then_converged();
    }

    #[test]
    fn a_request_starts_the_next_turn_on_the_same_change() {
        let requested = S::given(revised(10))
            .when(|task, _| task.request_turn("Add a test".to_owned()))
            .then([TaskEvent::TurnRequested {
                prompt: "Add a test".to_owned(),
            }]);
        assert_eq!(requested.state().phase(), TaskPhase::Working);
        requested
            .plan()
            .then_actions([TaskAction::StartTurn("Add a test".to_owned())]);
        requested
            .try_when(|task, _| task.turn_started(id(21), "Fix the bug".to_owned()))
            .then_error("task.out_of_order");
        let mut later = revised(10);
        later.extend([
            TaskEvent::TurnRequested {
                prompt: "Add a test".to_owned(),
            },
            TaskEvent::TurnStarted {
                job: id(21),
                prompt: "Add a test".to_owned(),
            },
            TaskEvent::TurnEnded {
                job: id(21),
                ending: exited(0),
            },
            TaskEvent::CommitsCollecting { job: id(31) },
            TaskEvent::CommitsCollected {
                job: id(31),
                ending: exited(0),
            },
        ]);
        S::given(later.clone())
            .try_when(|task, _| task.revised(id(51), 2))
            .then_error("task.out_of_order");
        S::given(later)
            .try_when(|task, _| task.revised(id(50), 2))
            .then([TaskEvent::Revised {
                change: id(50),
                revision: 2,
            }]);
    }

    #[test]
    fn a_failed_collection_is_retried_then_fails_the_task_keeping_its_sandbox() {
        let mut collecting = working(10);
        collecting.extend([
            TaskEvent::TurnEnded {
                job: id(20),
                ending: exited(0),
            },
            TaskEvent::CommitsCollecting { job: id(30) },
        ]);
        let retried = S::given(collecting.clone())
            .when(|task, _| task.job_ended(id(30), exited(128)))
            .then([TaskEvent::CommitsCollected {
                job: id(30),
                ending: exited(128),
            }]);
        retried.plan().then_actions([TaskAction::CollectCommits]);

        collecting.extend([
            TaskEvent::CommitsCollected {
                job: id(30),
                ending: exited(128),
            },
            TaskEvent::CommitsCollecting { job: id(31) },
            TaskEvent::CommitsCollected {
                job: id(31),
                ending: exited(128),
            },
            TaskEvent::CommitsCollecting { job: id(32) },
        ]);
        let failed = S::given(collecting)
            .when(|task, _| task.job_ended(id(32), exited(128)))
            .then([
                TaskEvent::CommitsCollected {
                    job: id(32),
                    ending: exited(128),
                },
                TaskEvent::Ended {
                    end: TaskEnd::Failed {
                        reason: "collecting the commits of turn 1 exited with 128, 3 times; the \
                                 sandbox is kept for recovery until the task is cancelled"
                            .to_owned(),
                    },
                },
            ]);
        failed.plan().then_converged();
    }

    #[test]
    fn the_change_title_is_the_goals_first_line_cut_at_a_word() {
        let task =
            |goal: &str| Task::new(id(1), id(2), goal.to_owned(), None, S::NOW).expect("task");
        assert_eq!(
            task("feat: diffs (P5.5)\n\nDetails").title(),
            "feat: diffs (P5.5)"
        );
        let long = task(&"word ".repeat(30)).title();
        assert!(long.chars().count() <= Task::TITLE_CHARS, "{long}");
        assert!(long.ends_with("word…"), "{long}");
    }

    #[test]
    fn a_turn_stopped_by_the_usage_limit_is_asked_again_later() {
        let limited = S::given(working(10))
            .when(|task, now| task.turn_limited(id(20), exited(1), now))
            .then([TaskEvent::TurnLimited {
                job: id(20),
                ending: exited(1),
                retry_at: S::NOW.saturating_add(Task::LIMIT_RETRY),
            }]);
        limited.plan().then_recheck(Task::LIMIT_RETRY);
        limited
            .after(Task::LIMIT_RETRY)
            .plan()
            .then_actions([TaskAction::StartTurn("Fix the bug".to_owned())]);
    }

    #[test]
    fn a_later_turn_stopped_by_the_usage_limit_keeps_its_prompt() {
        let mut events = working(10);
        events.extend([
            TaskEvent::TurnEnded {
                job: id(20),
                ending: exited(0),
            },
            TaskEvent::CommitsCollecting { job: id(30) },
            TaskEvent::CommitsCollected {
                job: id(30),
                ending: exited(0),
            },
            TaskEvent::Revised {
                change: id(40),
                revision: 1,
            },
            TaskEvent::TurnRequested {
                prompt: "Handle the empty case".to_owned(),
            },
            TaskEvent::TurnStarted {
                job: id(21),
                prompt: "Handle the empty case".to_owned(),
            },
        ]);
        S::given(events)
            .when(|task, now| task.turn_limited(id(21), exited(1), now))
            .then([TaskEvent::TurnLimited {
                job: id(21),
                ending: exited(1),
                retry_at: S::NOW.saturating_add(Task::LIMIT_RETRY),
            }])
            .after(Task::LIMIT_RETRY)
            .plan()
            .then_actions([TaskAction::StartTurn("Handle the empty case".to_owned())]);
    }

    #[test]
    fn a_task_whose_turns_keep_hitting_the_usage_limit_fails() {
        let mut events = working(10);
        for n in 0..Task::LIMIT_ATTEMPTS - 1 {
            let job = id(u128::from(20 + n));
            if n > 0 {
                events.push(TaskEvent::TurnStarted {
                    job,
                    prompt: "Fix the bug".to_owned(),
                });
            }
            events.push(TaskEvent::TurnLimited {
                job,
                ending: exited(1),
                retry_at: S::NOW,
            });
        }
        let last = id(u128::from(20 + Task::LIMIT_ATTEMPTS - 1));
        events.push(TaskEvent::TurnStarted {
            job: last,
            prompt: "Fix the bug".to_owned(),
        });
        S::given(events)
            .when(|task, now| task.turn_limited(last, exited(1), now))
            .then([
                TaskEvent::TurnEnded {
                    job: last,
                    ending: exited(1),
                },
                TaskEvent::Ended {
                    end: TaskEnd::Failed {
                        reason: "the tool's usage limit stopped its turns 12 times".to_owned(),
                    },
                },
            ]);
    }

    #[test]
    fn cancelling_a_task_that_kept_its_sandbox_releases_it() {
        let mut kept = working(10);
        kept.push(TaskEvent::TurnEnded {
            job: id(20),
            ending: exited(0),
        });
        for job in 30..33 {
            kept.extend([
                TaskEvent::CommitsCollecting { job: id(job) },
                TaskEvent::CommitsCollected {
                    job: id(job),
                    ending: exited(128),
                },
            ]);
        }
        kept.push(TaskEvent::Ended {
            end: TaskEnd::Failed {
                reason: "collecting failed".to_owned(),
            },
        });
        S::given(kept)
            .when(|task, _| task.cancel())
            .then([TaskEvent::SandboxReleased])
            .plan()
            .then_actions([TaskAction::StopSandbox(id(10))]);
    }

    #[test]
    fn a_task_over_its_max_turns_fails_and_stops_its_sandbox() {
        let failed = S::given(revised(1))
            .when(|task, _| task.request_turn("More".to_owned()))
            .then([TaskEvent::Ended {
                end: TaskEnd::Failed {
                    reason: "the task took its 1 turns".to_owned(),
                },
            }]);
        failed
            .plan()
            .then_actions([TaskAction::StopSandbox(id(10))]);
        failed
            .when(|task, _| task.sandbox_stopped())
            .then([TaskEvent::SandboxStopped])
            .plan()
            .then_converged();
    }

    #[test]
    fn a_failed_turn_fails_the_task() {
        let lost = JobEnding::Failed {
            reason: JobFailure::TimedOut,
        };
        S::given(working(10))
            .when(|task, _| task.job_ended(id(20), lost))
            .then([
                TaskEvent::TurnEnded {
                    job: id(20),
                    ending: lost,
                },
                TaskEvent::Ended {
                    end: TaskEnd::Failed {
                        reason: "turn 1 did not complete: timed out".to_owned(),
                    },
                },
            ]);
    }

    #[test]
    fn a_cancelled_task_stops_its_sandbox_and_ignores_later_steps() {
        let cancelled =
            S::given(working(10))
                .when(|task, _| task.cancel())
                .then([TaskEvent::Ended {
                    end: TaskEnd::Cancelled,
                }]);
        cancelled
            .plan()
            .then_actions([TaskAction::StopSandbox(id(10))]);
        cancelled
            .when(|task, _| task.job_ended(id(20), exited(0)))
            .then_no_events()
            .when(|task, _| task.fail("late".to_owned()))
            .then_no_events()
            .when(|task, _| task.request_turn("More".to_owned()))
            .then_no_events()
            .when(|task, _| task.finish())
            .then_no_events();
    }

    #[test]
    fn a_request_during_a_turn_waits_for_its_revision_and_requests_add_up() {
        let queued = S::given(working(10))
            .when(|task, _| task.request_turn("First".to_owned()))
            .then([TaskEvent::TurnRequested {
                prompt: "First".to_owned(),
            }])
            .when(|task, _| task.request_turn("Second".to_owned()))
            .then([TaskEvent::TurnRequested {
                prompt: "Second".to_owned(),
            }]);
        queued.plan().then_converged();
        let mut done = revised(10);
        done.extend([
            TaskEvent::TurnRequested {
                prompt: "First".to_owned(),
            },
            TaskEvent::TurnRequested {
                prompt: "Second".to_owned(),
            },
        ]);
        S::given(done)
            .plan()
            .then_actions([TaskAction::StartTurn("First\n\nSecond".to_owned())]);
    }

    #[test]
    fn a_merged_change_finishes_the_task() {
        let done = S::given(revised(10))
            .when(|task, _| task.finish())
            .then([TaskEvent::Ended { end: TaskEnd::Done }]);
        done.plan().then_actions([TaskAction::StopSandbox(id(10))]);
    }

    #[test]
    fn a_review_prompt_lists_the_comments_with_their_place() {
        let reviewer = igloo_core::Actor::Human { user: id(7) };
        let comment =
            |path: Option<&str>, line: Option<u32>, body: &str| igloo_core::change::Comment {
                revision: 2,
                author: reviewer,
                body: body.to_owned(),
                path: path.map(str::to_owned),
                line,
                at: S::NOW,
            };
        let request = ChangeRequest {
            revision: 2,
            by: reviewer,
            comments: vec![
                comment(Some("src/lib.rs"), Some(3), "Handle the empty case"),
                comment(Some("README.md"), None, "Document it"),
                comment(None, None, "Add a test"),
            ],
            at: S::NOW,
        };
        assert_eq!(
            Task::review_prompt(&request),
            "A reviewer asked for changes to revision 2:\n\n\
             - src/lib.rs:3: Handle the empty case\n\
             - README.md: Document it\n\
             - Add a test\n\n\
             Address every comment."
        );
    }

    #[test]
    fn steps_out_of_order_are_rejected() {
        S::given([created()])
            .try_when(|task, _| task.ready(snapshot(3)))
            .then_error("task.out_of_order");
        S::given([created()])
            .try_when(|task, _| task.sandbox_started(id(10)))
            .then_error("task.out_of_order");
        S::given(working(10))
            .try_when(|task, _| task.prepared(prepared(10)))
            .then_error("task.out_of_order");
    }

    #[test]
    fn events_serialize_stably() {
        insta::assert_json_snapshot!(working(10));
    }
}
