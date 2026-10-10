//! Runs: the checks of a change's revision.

use igloo_core::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Where a run is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RunPhase {
    /// Reading the pipeline and preparing the snapshot.
    Preparing,
    /// Building the warm snapshot.
    Warming,
    /// Running the checks.
    Checking,
    /// Every check passed.
    Passed,
    /// A check failed or did not complete.
    Failed,
    /// The run could not check; see `error`.
    Errored,
}

/// Where a check is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CheckStatus {
    /// Not submitted yet.
    Pending,
    /// Submitted; see `job` for its output.
    Started,
    /// Exited 0.
    Passed,
    /// Exited with another code; see `exit_code`.
    Failed,
    /// Did not complete; see `reason`.
    Errored,
}

/// One check of a run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct CheckResource {
    /// Its name in the pipeline.
    pub name: String,
    /// Where it is.
    pub status: CheckStatus,
    /// The job running it, whose logs are its output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// The exit code, when it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Why it did not complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// When its job's process started; absent until it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub started_at: Option<Timestamp>,
    /// When its job ended; absent until it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub ended_at: Option<Timestamp>,
}

/// A run of a change's revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct RunResource {
    /// Its id (`run_...`).
    pub id: String,
    /// The change.
    pub change: String,
    /// The revision's number.
    pub revision: u32,
    /// The commit checked.
    pub commit: String,
    /// Where it is.
    pub phase: RunPhase,
    /// Why it errored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The job of the warm build this run did, if any; its logs are the build's output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warm_job: Option<String>,
    /// Its checks, in pipeline order; empty until prepared.
    pub checks: Vec<CheckResource>,
    /// When it started.
    #[schema(value_type = String, format = DateTime)]
    pub started_at: Timestamp,
    /// When it ended, passed, failed or errored; absent while it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub ended_at: Option<Timestamp>,
    /// The sandbox its checks run in; absent until they start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
}

/// A page of runs, newest first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct RunList {
    /// The runs, newest first.
    pub items: Vec<RunResource>,
    /// Pass as `cursor` to get the next page; absent on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

impl RunList {
    /// A page of `items`, followed by the page after `next_cursor` if any.
    #[must_use]
    pub const fn new(items: Vec<RunResource>, next_cursor: Option<String>) -> Self {
        Self { items, next_cursor }
    }
}

impl CheckResource {
    /// Check `name` at `status`.
    #[must_use]
    pub const fn new(name: String, status: CheckStatus) -> Self {
        Self {
            name,
            status,
            job: None,
            exit_code: None,
            reason: None,
            started_at: None,
            ended_at: None,
        }
    }

    /// Sets when its job's process started.
    #[must_use]
    pub const fn with_started_at(mut self, at: Timestamp) -> Self {
        self.started_at = Some(at);
        self
    }

    /// Sets when its job ended.
    #[must_use]
    pub const fn with_ended_at(mut self, at: Timestamp) -> Self {
        self.ended_at = Some(at);
        self
    }

    /// Sets the job.
    #[must_use]
    pub fn with_job(mut self, job: impl Into<String>) -> Self {
        self.job = Some(job.into());
        self
    }

    /// Sets the exit code.
    #[must_use]
    pub const fn with_exit_code(mut self, exit_code: i32) -> Self {
        self.exit_code = Some(exit_code);
        self
    }

    /// Sets why it did not complete.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

impl RunResource {
    /// Run `id` of `change`'s `revision` at `commit`.
    #[must_use]
    pub fn new(
        id: String,
        (change, revision): (String, u32),
        commit: String,
        phase: RunPhase,
        started_at: Timestamp,
    ) -> Self {
        Self {
            id,
            change,
            revision,
            commit,
            phase,
            error: None,
            warm_job: None,
            checks: Vec::new(),
            started_at,
            ended_at: None,
            sandbox: None,
        }
    }

    /// Sets when it ended.
    #[must_use]
    pub const fn with_ended_at(mut self, at: Timestamp) -> Self {
        self.ended_at = Some(at);
        self
    }

    /// Sets the sandbox of its checks.
    #[must_use]
    pub fn with_sandbox(mut self, sandbox: impl Into<String>) -> Self {
        self.sandbox = Some(sandbox.into());
        self
    }

    /// Sets why it errored.
    #[must_use]
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    /// Sets the warm build's job.
    #[must_use]
    pub fn with_warm_job(mut self, job: impl Into<String>) -> Self {
        self.warm_job = Some(job.into());
        self
    }

    /// Sets the checks.
    #[must_use]
    pub fn with_checks(mut self, checks: Vec<CheckResource>) -> Self {
        self.checks = checks;
        self
    }
}
