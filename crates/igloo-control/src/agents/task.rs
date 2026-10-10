use igloo_core::build::BuildId;
use igloo_core::change::{ChangeId, ChangeRequest};
use igloo_core::job::{JobEnding, JobId};
use igloo_core::repo::{CommitId, RepoId};
use igloo_core::sandbox::SandboxId;
use igloo_core::snapshot::SnapshotId;
use igloo_core::{
    Actor, Entity, ErrorCode, Event, Generation, Id, Labels, Plan, Prefixed, Resource, Timestamp,
    UserId,
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
/// at most the tool's `max_turns` run; the sandbox is stopped once the task ends. While a
/// person has taken the task over no turn starts, and the sandbox stays running until the task
/// ends.
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
    takeover: Option<Takeover>,
    change: Option<ChangeId>,
    end: Option<TaskEnd>,
    created_at: Timestamp,
    events: Vec<TaskEvent>,
}

/// A person's hold on a task's sandbox, and how far handing it back is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Takeover {
    by: UserId,
    step: Handback,
}

/// How far handing a taken-over task back is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handback {
    /// The person holds the sandbox.
    Held,
    /// The person handed the task back; the sandbox is to be read.
    Requested,
    /// This job reads what the person changed.
    Reading(JobId),
    /// The reading job ended; the prompt is to be recorded.
    Read { job: JobId, ending: JobEnding },
}

/// Where a takeover is, as reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeoverPhase {
    /// A turn or the collection of its commits is still in flight; it is not interrupted and
    /// no further turn starts.
    Waiting,
    /// No turn runs; the person holds the sandbox and may hand the task back.
    Paused,
    /// The person handed the task back; the sandbox is read for what they changed.
    HandingBack,
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
    /// Start the job reading what the person who took the task over changed in the sandbox.
    ReadChanges,
    /// Ask the tool to continue, naming what the person changed as `job` read it; `read` is
    /// whether the job succeeded.
    HandBack {
        /// The reading job.
        job: JobId,
        /// Whether it succeeded.
        read: bool,
    },
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
    /// A person took the task over: its terminal is writable for them and no turn starts.
    TakenOver {
        /// The person.
        by: UserId,
    },
    /// The person handed the task back; what they changed is to be read in the sandbox.
    HandBackRequested,
    /// This job reads what the person changed.
    ChangesReading {
        /// The reading job.
        job: JobId,
    },
    /// The reading job ended.
    ChangesRead {
        /// The reading job.
        job: JobId,
        /// How.
        ending: JobEnding,
    },
    /// The task was handed back: the next turn is asked `prompt`, which names what the person
    /// changed.
    HandedBack {
        /// What the tool is asked.
        prompt: String,
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
    /// Only a person takes a task over or hands it back.
    #[error("only a person can take a task over or hand it back")]
    NotAPerson,
    /// Another person holds the task.
    #[error("the task is taken over by someone else")]
    AlreadyTakenOver,
    /// Nobody has taken the task over.
    #[error("the task is not taken over")]
    NotTakenOver,
    /// Only the person who took the task over hands it back or types in its terminal.
    #[error("the task was taken over by someone else")]
    NotYourTakeover,
    /// The person handed the task back; its sandbox is no longer theirs to type in.
    #[error("the task is being handed back")]
    HandingBack,
    /// A turn is still in flight; the task can be handed back once it ended and its commits
    /// became a revision.
    #[error("a turn is still running")]
    TurnRunning,
}

impl ErrorCode for TaskError {
    fn code(&self) -> &'static str {
        match self {
            Self::OutOfOrder => "task.out_of_order",
            Self::EmptyGoal => "task.empty_goal",
            Self::NotAPerson => "task.not_a_person",
            Self::AlreadyTakenOver => "task.already_taken_over",
            Self::NotTakenOver => "task.not_taken_over",
            Self::NotYourTakeover => "task.not_your_takeover",
            Self::HandingBack => "task.handing_back",
            Self::TurnRunning => "task.turn_running",
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

    /// Takes the task over for the person `by`: their terminal in the sandbox becomes writable
    /// and no turn starts until they hand the task back. A running turn is not interrupted.
    /// Taking over a task the person already holds records nothing.
    pub fn take_over(&mut self, by: Actor) -> Result<(), TaskError> {
        let Actor::Human { user } = by else {
            return Err(TaskError::NotAPerson);
        };
        if self.end.is_some() || self.sandbox.is_none() || self.stopped {
            return Err(TaskError::OutOfOrder);
        }
        match self.takeover {
            Some(Takeover { by, .. }) if by == user => Ok(()),
            Some(_) => Err(TaskError::AlreadyTakenOver),
            None => {
                self.record(TaskEvent::TakenOver { by: user });
                Ok(())
            }
        }
    }

    /// Hands the task back for the person who took it over, once no turn is in flight: the
    /// sandbox is read for what they changed, then the next turn is asked about it. Handing
    /// back a task that is being handed back records nothing.
    pub fn hand_back(&mut self, by: Actor) -> Result<(), TaskError> {
        let Actor::Human { user } = by else {
            return Err(TaskError::NotAPerson);
        };
        let Some(Takeover { by: holder, step }) = self.takeover else {
            return Err(TaskError::NotTakenOver);
        };
        if holder != user {
            return Err(TaskError::NotYourTakeover);
        }
        if step != Handback::Held {
            return Ok(());
        }
        if !self.quiet() {
            return Err(TaskError::TurnRunning);
        }
        self.record(TaskEvent::HandBackRequested);
        Ok(())
    }

    /// Whether `by` may type in the task's sandbox: they took the task over and have not
    /// handed it back.
    pub fn writable_by(&self, by: Actor) -> Result<(), TaskError> {
        let Actor::Human { user } = by else {
            return Err(TaskError::NotAPerson);
        };
        match self.takeover {
            None => Err(TaskError::NotTakenOver),
            Some(Takeover { by, .. }) if by != user => Err(TaskError::NotYourTakeover),
            Some(Takeover {
                step: Handback::Held,
                ..
            }) => Ok(()),
            Some(_) => Err(TaskError::HandingBack),
        }
    }

    /// Records the job reading what the person changed.
    pub fn changes_reading(&mut self, job: JobId) -> Result<(), TaskError> {
        if self.end.is_some() || self.step() != Some(Handback::Requested) {
            return Err(TaskError::OutOfOrder);
        }
        self.record(TaskEvent::ChangesReading { job });
        Ok(())
    }

    /// Hands the task back with `prompt`, which names what the person changed: nobody holds the
    /// task any more and its next turn is asked `prompt`, then whatever was requested meanwhile.
    /// A task that already took its tool's `max_turns` fails instead.
    pub fn handed_back(&mut self, prompt: String) -> Result<(), TaskError> {
        if self.end.is_some() || !matches!(self.step(), Some(Handback::Read { .. })) {
            return Err(TaskError::OutOfOrder);
        }
        let max = self
            .prepared
            .as_ref()
            .map_or(u32::MAX, |prepared| prepared.spec.max_turns);
        let turns = u32::try_from(self.turns.len()).unwrap_or(u32::MAX);
        let over = self.requested.is_none() && turns >= max;
        self.record(TaskEvent::HandedBack { prompt });
        if over {
            self.end_with(TaskEnd::Failed {
                reason: format!("the task took its {max} turns"),
            });
        }
        Ok(())
    }

    /// The person who took the task over, until it is handed back.
    #[must_use]
    pub fn taken_over_by(&self) -> Option<UserId> {
        self.takeover.map(|takeover| takeover.by)
    }

    /// Where the takeover is, if the task is taken over.
    #[must_use]
    pub fn takeover(&self) -> Option<TakeoverPhase> {
        let takeover = self.takeover?;
        Some(match takeover.step {
            Handback::Held if self.quiet() => TakeoverPhase::Paused,
            Handback::Held => TakeoverPhase::Waiting,
            _ => TakeoverPhase::HandingBack,
        })
    }

    /// The job reading what the person changed, once started.
    #[must_use]
    pub fn changes_job(&self) -> Option<JobId> {
        match self.step()? {
            Handback::Reading(job) | Handback::Read { job, .. } => Some(job),
            Handback::Held | Handback::Requested => None,
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

    /// Records how a job ended; only the last turn's job, its collecting job and the job
    /// reading a handed-back sandbox count, once each. A failed turn fails the task. A failed collection is retried; once
    /// [`Task::COLLECT_ATTEMPTS`] have failed, the task fails and keeps its sandbox, so the
    /// turn's commits stay recoverable, until it is cancelled.
    pub fn job_ended(&mut self, job: JobId, ending: JobEnding) {
        if self.end.is_none() && self.step() == Some(Handback::Reading(job)) {
            self.record(TaskEvent::ChangesRead { job, ending });
            return;
        }
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
        } else if self.due_prompt().is_none()
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

    /// The prompt of the turn to start next, if one is due and nobody holds the task.
    fn next_prompt(&self) -> Option<String> {
        if self.takeover.is_some() {
            return None;
        }
        self.due_prompt()
    }

    /// The prompt of the next turn, whoever holds the task: the goal first, then requests.
    fn due_prompt(&self) -> Option<String> {
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

    fn step(&self) -> Option<Handback> {
        self.takeover.map(|takeover| takeover.step)
    }

    /// Whether no turn is in flight: the last turn, if any, became a revision.
    fn quiet(&self) -> bool {
        self.turns.last().is_none_or(|turn| turn.revision.is_some())
    }

    fn set_step(&mut self, step: Handback) {
        if let Some(takeover) = &mut self.takeover {
            takeover.step = step;
        }
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
            takeover: None,
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
            Self::TakenOver { .. } => "igloo.task.taken_over",
            Self::HandBackRequested => "igloo.task.hand_back_requested",
            Self::ChangesReading { .. } => "igloo.task.changes_reading",
            Self::ChangesRead { .. } => "igloo.task.changes_read",
            Self::HandedBack { .. } => "igloo.task.handed_back",
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
            TaskEvent::TakenOver { by } => {
                self.takeover = Some(Takeover {
                    by: *by,
                    step: Handback::Held,
                });
            }
            TaskEvent::HandBackRequested => self.set_step(Handback::Requested),
            TaskEvent::ChangesReading { job } => self.set_step(Handback::Reading(*job)),
            TaskEvent::ChangesRead { job, ending } => self.set_step(Handback::Read {
                job: *job,
                ending: *ending,
            }),
            TaskEvent::HandedBack { prompt } => {
                self.takeover = None;
                self.requested = Some(match self.requested.take() {
                    Some(pending) => format!("{prompt}\n\n{pending}"),
                    None => prompt.clone(),
                });
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
    /// recorded. A taken-over task starts no turn but still collects and publishes the commits
    /// of the turn that ended. An ended task stops its sandbox, unless it keeps it for
    /// recovery.
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
                match self.step() {
                    Some(Handback::Requested) => return Plan::Act(vec![TaskAction::ReadChanges]),
                    Some(Handback::Reading(_)) => return Plan::Converged,
                    Some(Handback::Read { job, ending }) => {
                        return Plan::Act(vec![TaskAction::HandBack {
                            job,
                            read: ending.succeeded(),
                        }]);
                    }
                    Some(Handback::Held) | None => {}
                }
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

    fn person(n: u128) -> Actor {
        Actor::Human { user: id(n) }
    }

    fn agent_actor() -> Actor {
        Actor::Agent {
            agent: id(5),
            principal: id(7),
        }
    }

    /// `events` followed by person 7 taking the task over.
    fn taken_over(mut events: Vec<TaskEvent>) -> Vec<TaskEvent> {
        events.push(TaskEvent::TakenOver { by: id(7) });
        events
    }

    /// A taken-over task whose person handed it back and whose sandbox was read.
    fn read(max_turns: u32) -> Vec<TaskEvent> {
        let mut events = taken_over(revised(max_turns));
        events.extend([
            TaskEvent::HandBackRequested,
            TaskEvent::ChangesReading { job: id(60) },
            TaskEvent::ChangesRead {
                job: id(60),
                ending: exited(0),
            },
        ]);
        events
    }

    #[test]
    fn only_a_person_takes_a_task_over_and_only_once() {
        S::given(working(10))
            .try_when(|task, _| task.take_over(agent_actor()))
            .then_error("task.not_a_person");
        S::given(working(10))
            .try_when(|task, _| {
                task.take_over(Actor::System {
                    component: igloo_core::SystemComponent::Controller,
                })
            })
            .then_error("task.not_a_person");
        let held = S::given(working(10))
            .try_when(|task, _| task.take_over(person(7)))
            .then([TaskEvent::TakenOver { by: id(7) }]);
        assert_eq!(held.state().taken_over_by(), Some(id(7)));
        S::given(taken_over(working(10)))
            .try_when(|task, _| task.take_over(person(7)))
            .then_no_events();
        S::given(taken_over(working(10)))
            .try_when(|task, _| task.take_over(person(8)))
            .then_error("task.already_taken_over");
        S::given([created()])
            .try_when(|task, _| task.take_over(person(7)))
            .then_error("task.out_of_order");
    }

    #[test]
    fn a_running_turn_is_not_interrupted_but_no_turn_starts_while_taken_over() {
        let held = S::given(taken_over(working(10)));
        assert_eq!(held.state().takeover(), Some(TakeoverPhase::Waiting));
        held.plan().then_converged();
        let ended = held
            .when(|task, _| task.job_ended(id(20), exited(0)))
            .then([TaskEvent::TurnEnded {
                job: id(20),
                ending: exited(0),
            }]);
        ended.plan().then_actions([TaskAction::CollectCommits]);

        let mut events = taken_over(revised(10));
        events.push(TaskEvent::TurnRequested {
            prompt: "Add a test".to_owned(),
        });
        let paused = S::given(events);
        assert_eq!(paused.state().takeover(), Some(TakeoverPhase::Paused));
        assert_eq!(paused.state().phase(), TaskPhase::Working);
        paused.plan().then_converged();
        paused
            .try_when(|task, _| task.turn_started(id(21), "Add a test".to_owned()))
            .then_error("task.out_of_order");
    }

    #[test]
    fn a_task_taken_over_before_its_first_turn_starts_none() {
        let mut events = working(10);
        events.pop();
        let held = S::given(taken_over(events));
        held.plan().then_converged();
        assert_eq!(held.state().takeover(), Some(TakeoverPhase::Paused));
    }

    #[test]
    fn handing_back_needs_the_holder_and_a_turn_that_ended() {
        S::given(taken_over(working(10)))
            .try_when(|task, _| task.hand_back(person(7)))
            .then_error("task.turn_running");
        S::given(taken_over(revised(10)))
            .try_when(|task, _| task.hand_back(person(8)))
            .then_error("task.not_your_takeover");
        S::given(taken_over(revised(10)))
            .try_when(|task, _| task.hand_back(agent_actor()))
            .then_error("task.not_a_person");
        S::given(revised(10))
            .try_when(|task, _| task.hand_back(person(7)))
            .then_error("task.not_taken_over");
    }

    #[test]
    fn handing_back_reads_the_sandbox_then_asks_for_a_turn() {
        let requested = S::given(taken_over(revised(10)))
            .try_when(|task, _| task.hand_back(person(7)))
            .then([TaskEvent::HandBackRequested]);
        assert_eq!(
            requested.state().takeover(),
            Some(TakeoverPhase::HandingBack)
        );
        requested.plan().then_actions([TaskAction::ReadChanges]);
        let reading = requested
            .try_when(|task, _| task.changes_reading(id(60)))
            .then([TaskEvent::ChangesReading { job: id(60) }]);
        assert_eq!(reading.state().changes_job(), Some(id(60)));
        reading.plan().then_converged();

        let read = reading
            .when(|task, _| task.job_ended(id(60), exited(0)))
            .then([TaskEvent::ChangesRead {
                job: id(60),
                ending: exited(0),
            }]);
        read.plan().then_actions([TaskAction::HandBack {
            job: id(60),
            read: true,
        }]);
        let back = read
            .try_when(|task, _| task.handed_back("They fixed the typo.".to_owned()))
            .then([TaskEvent::HandedBack {
                prompt: "They fixed the typo.".to_owned(),
            }]);
        assert_eq!(back.state().taken_over_by(), None);
        assert_eq!(back.state().takeover(), None);
        assert_eq!(back.state().phase(), TaskPhase::Working);
        back.plan()
            .then_actions([TaskAction::StartTurn("They fixed the typo.".to_owned())]);
    }

    #[test]
    fn each_step_of_handing_back_happens_once_and_in_order() {
        let handing = || {
            let mut events = taken_over(revised(10));
            events.push(TaskEvent::HandBackRequested);
            events
        };
        S::given(handing())
            .try_when(|task, _| task.hand_back(person(7)))
            .then_no_events();
        S::given(handing())
            .try_when(|task, _| task.handed_back("early".to_owned()))
            .then_error("task.out_of_order");
        let mut reading = handing();
        reading.push(TaskEvent::ChangesReading { job: id(60) });
        S::given(reading.clone())
            .try_when(|task, _| task.changes_reading(id(61)))
            .then_error("task.out_of_order");
        S::given(reading)
            .when(|task, _| task.job_ended(id(99), exited(0)))
            .then_no_events();
        S::given(read(10))
            .when(|task, _| task.job_ended(id(60), exited(0)))
            .then_no_events();
        S::given(taken_over(revised(10)))
            .try_when(|task, _| task.changes_reading(id(60)))
            .then_error("task.out_of_order");
    }

    #[test]
    fn a_failed_reading_still_hands_the_task_back() {
        let mut events = taken_over(revised(10));
        events.extend([
            TaskEvent::HandBackRequested,
            TaskEvent::ChangesReading { job: id(60) },
        ]);
        S::given(events)
            .when(|task, _| task.job_ended(id(60), exited(1)))
            .then([TaskEvent::ChangesRead {
                job: id(60),
                ending: exited(1),
            }])
            .plan()
            .then_actions([TaskAction::HandBack {
                job: id(60),
                read: false,
            }]);
    }

    #[test]
    fn a_request_made_while_taken_over_follows_the_handed_back_prompt() {
        let mut events = read(10);
        events.insert(
            events.len() - 3,
            TaskEvent::TurnRequested {
                prompt: "Add a test".to_owned(),
            },
        );
        S::given(events)
            .try_when(|task, _| task.handed_back("They edited.".to_owned()))
            .then([TaskEvent::HandedBack {
                prompt: "They edited.".to_owned(),
            }])
            .plan()
            .then_actions([TaskAction::StartTurn(
                "They edited.\n\nAdd a test".to_owned(),
            )]);
    }

    #[test]
    fn a_task_over_its_max_turns_fails_when_handed_back() {
        let failed = S::given(read(1))
            .try_when(|task, _| task.handed_back("They edited.".to_owned()))
            .then([
                TaskEvent::HandedBack {
                    prompt: "They edited.".to_owned(),
                },
                TaskEvent::Ended {
                    end: TaskEnd::Failed {
                        reason: "the task took its 1 turns".to_owned(),
                    },
                },
            ]);
        failed
            .plan()
            .then_actions([TaskAction::StopSandbox(id(10))]);
    }

    #[test]
    fn only_the_holder_types_until_the_task_is_handed_back() {
        let held = || S::given(taken_over(revised(10)));
        held().plan().then_converged();
        held()
            .state()
            .writable_by(person(7))
            .expect("the holder types");
        assert_eq!(
            held().state().writable_by(person(8)),
            Err(TaskError::NotYourTakeover)
        );
        assert_eq!(
            held().state().writable_by(agent_actor()),
            Err(TaskError::NotAPerson)
        );
        assert_eq!(
            S::given(revised(10)).state().writable_by(person(7)),
            Err(TaskError::NotTakenOver)
        );
        let handing = held()
            .try_when(|task, _| task.hand_back(person(7)))
            .then([TaskEvent::HandBackRequested]);
        assert_eq!(
            handing.state().writable_by(person(7)),
            Err(TaskError::HandingBack)
        );
    }

    #[test]
    fn a_taken_over_task_that_ends_stops_its_sandbox() {
        let cancelled = S::given(taken_over(revised(10)))
            .when(|task, _| task.cancel())
            .then([TaskEvent::Ended {
                end: TaskEnd::Cancelled,
            }]);
        cancelled
            .plan()
            .then_actions([TaskAction::StopSandbox(id(10))]);
        cancelled
            .try_when(|task, _| task.take_over(person(7)))
            .then_error("task.out_of_order");
    }

    #[test]
    fn events_serialize_stably() {
        insta::assert_json_snapshot!(working(10));
    }

    #[test]
    fn takeover_events_serialize_stably() {
        insta::assert_json_snapshot!([
            TaskEvent::TakenOver { by: id(7) },
            TaskEvent::HandBackRequested,
            TaskEvent::ChangesReading { job: id(60) },
            TaskEvent::ChangesRead {
                job: id(60),
                ending: exited(0),
            },
            TaskEvent::HandedBack {
                prompt: "They fixed the typo.".to_owned(),
            },
        ]);
    }
}
