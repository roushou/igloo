//! Tasks: one agent on one goal in one sandbox, working in turns.

use std::str::FromStr;

use igloo_core::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::list::UnknownValue;

/// Creates a task on a repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct CreateTaskRequest {
    /// What the agent is asked to do; not empty. Its first line titles the task's change.
    pub goal: String,
    /// A tool of the repository's `.igloo/agents.toml`; its default tool when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

/// Where a task is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TaskPhase {
    /// Reading its settings and building its snapshot.
    Preparing,
    /// A turn runs, or is about to.
    Working,
    /// Its last turn ended; waiting for a review.
    AwaitingReview,
    /// Its change merged.
    Done,
    /// It could not continue; see `error`.
    Failed,
    /// Someone cancelled it.
    Cancelled,
}

/// Where a turn is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TurnStatus {
    /// The tool works; see `job` for its output.
    Running,
    /// The tool exited 0.
    Succeeded,
    /// The tool exited with another code or did not complete; see `reason`.
    Failed,
}

/// One turn of a task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct TurnResource {
    /// Its position, from 1.
    pub number: u32,
    /// The job running the tool, whose logs are its output.
    pub job: String,
    /// What the tool was asked.
    pub prompt: String,
    /// Where it is.
    pub status: TurnStatus,
    /// How it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct TaskResource {
    /// Its id (`task_...`).
    pub id: String,
    /// The repository.
    pub repo: String,
    /// What the agent is asked to do.
    pub goal: String,
    /// The tool, once settled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Where it is.
    pub phase: TaskPhase,
    /// The commit it started from, once settled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Its sandbox, once started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
    /// The change its commits form, once it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<String>,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Its turns, oldest first.
    pub turns: Vec<TurnResource>,
    /// When it was created.
    #[schema(value_type = String, format = DateTime)]
    pub created_at: Timestamp,
}

impl CreateTaskRequest {
    /// A request for `goal` with the repository's default tool.
    #[must_use]
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            tool: None,
        }
    }

    /// Asks for `tool` instead of the default.
    #[must_use]
    pub fn with_tool(mut self, tool: impl Into<String>) -> Self {
        self.tool = Some(tool.into());
        self
    }
}

impl TurnResource {
    /// Turn `number`, run as `job` with `prompt`, at `status`.
    #[must_use]
    pub const fn new(number: u32, job: String, prompt: String, status: TurnStatus) -> Self {
        Self {
            number,
            job,
            prompt,
            status,
            reason: None,
        }
    }

    /// Sets how it failed.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

impl TaskResource {
    /// Task `id` of `repo` toward `goal`, at `phase`.
    #[must_use]
    pub const fn new(
        id: String,
        repo: String,
        goal: String,
        phase: TaskPhase,
        created_at: Timestamp,
    ) -> Self {
        Self {
            id,
            repo,
            goal,
            tool: None,
            phase,
            commit: None,
            sandbox: None,
            change: None,
            error: None,
            turns: Vec::new(),
            created_at,
        }
    }

    /// Sets the tool and the commit it started from.
    #[must_use]
    pub fn with_settings(mut self, tool: impl Into<String>, commit: impl Into<String>) -> Self {
        self.tool = Some(tool.into());
        self.commit = Some(commit.into());
        self
    }

    /// Sets the sandbox.
    #[must_use]
    pub fn with_sandbox(mut self, sandbox: impl Into<String>) -> Self {
        self.sandbox = Some(sandbox.into());
        self
    }

    /// Sets the change.
    #[must_use]
    pub fn with_change(mut self, change: impl Into<String>) -> Self {
        self.change = Some(change.into());
        self
    }

    /// Sets why it failed.
    #[must_use]
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    /// Sets the turns.
    #[must_use]
    pub fn with_turns(mut self, turns: Vec<TurnResource>) -> Self {
        self.turns = turns;
        self
    }
}

/// One thing a tool did or said.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TranscriptItem {
    /// Text the tool wrote to the reader.
    Message {
        /// The text.
        text: String,
    },
    /// The tool called one of its tools.
    ToolCall {
        /// The call's id, matching its result.
        id: String,
        /// The tool called, such as `Bash`.
        name: String,
        /// Its input, as JSON.
        input: String,
    },
    /// What a tool call returned.
    ToolResult {
        /// The call's id.
        id: String,
        /// The output.
        output: String,
        /// Whether the call failed.
        is_error: bool,
    },
    /// A line of output the tool's harness does not read further.
    Output {
        /// The line.
        text: String,
    },
    /// The tool reported a failure.
    Error {
        /// What it said.
        text: String,
    },
}

/// An entry of a task's transcript.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct TranscriptEntry {
    /// Its position in the transcript, from 0.
    pub position: u32,
    /// The turn it belongs to, from 1.
    pub turn: u32,
    /// What happened.
    pub item: TranscriptItem,
}

/// A task's transcript from a position on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct TranscriptResource {
    /// The entries, oldest first.
    pub entries: Vec<TranscriptEntry>,
    /// The position to read from next.
    pub next: u32,
    /// Whether no turn runs, so no entries follow until another turn starts.
    pub idle: bool,
}

impl TranscriptEntry {
    /// `item` at `position`, in turn `turn`.
    #[must_use]
    pub const fn new(position: u32, turn: u32, item: TranscriptItem) -> Self {
        Self {
            position,
            turn,
            item,
        }
    }
}

impl TranscriptResource {
    /// `entries`, read on from `next`; `idle` when no turn runs.
    #[must_use]
    pub const fn new(entries: Vec<TranscriptEntry>, next: u32, idle: bool) -> Self {
        Self {
            entries,
            next,
            idle,
        }
    }
}

impl FromStr for TaskPhase {
    type Err = UnknownValue;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "preparing" => Ok(Self::Preparing),
            "working" => Ok(Self::Working),
            "awaiting_review" => Ok(Self::AwaitingReview),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(UnknownValue(other.to_owned())),
        }
    }
}
