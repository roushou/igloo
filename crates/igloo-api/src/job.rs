//! Jobs: processes run in a sandbox.

use std::collections::{BTreeMap, BTreeSet};

use igloo_core::job::{self as domain, Job, JobSpec, JobTimeout};
use igloo_core::process::{Argv, EnvVars};
use igloo_core::repo::SecretName;
use igloo_core::sandbox::SandboxId;
use igloo_core::{Entity, Timestamp, ValidationErrors, Validator};
use jiff::SignedDuration;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Runs a process in a sandbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct ExecRequest {
    /// The program and its arguments.
    pub argv: Vec<String>,
    /// Extra environment, layered over the sandbox's.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Secrets of the sandbox's repository to expose as environment variables, by name.
    #[serde(default)]
    pub secrets: Vec<String>,
    /// How long the process may run, 1 to 86400 seconds; one hour when omitted.
    #[serde(default)]
    pub timeout_seconds: Option<u32>,
}

/// Where a job is in its lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum JobPhase {
    /// Waiting for its worker.
    Queued,
    /// Claimed by its worker.
    Leased,
    /// Running.
    Running,
    /// The process exited; see `exit_code`.
    Finished,
    /// The job could not complete; see `failure_reason`.
    Failed,
    /// Cancelled.
    Cancelled,
}

/// A job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct JobResource {
    /// Its id (`job_...`).
    pub id: String,
    /// The sandbox it runs in.
    pub sandbox: String,
    /// The program and its arguments.
    pub argv: Vec<String>,
    /// Extra environment.
    pub env: BTreeMap<String, String>,
    /// The repository secrets it is given, by name.
    #[serde(default)]
    pub secrets: Vec<String>,
    /// How long the process may run.
    pub timeout_seconds: u64,
    /// The phase.
    pub phase: JobPhase,
    /// The exit code, when finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Why it failed, when failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// When it was submitted.
    #[schema(value_type = String, format = DateTime)]
    pub submitted_at: Timestamp,
}

impl ExecRequest {
    /// A request to run `argv` with no extra environment and the default timeout.
    #[must_use]
    pub fn new(argv: Vec<String>) -> Self {
        Self {
            argv,
            env: BTreeMap::new(),
            secrets: Vec::new(),
            timeout_seconds: None,
        }
    }

    /// Exposes the repository secrets `names`.
    #[must_use]
    pub fn with_secrets(mut self, names: Vec<String>) -> Self {
        self.secrets = names;
        self
    }

    /// Sets the timeout.
    #[must_use]
    pub const fn with_timeout_seconds(mut self, seconds: u32) -> Self {
        self.timeout_seconds = Some(seconds);
        self
    }

    /// The job spec for running this request in `sandbox`, reporting every invalid field.
    pub fn into_spec(self, sandbox: SandboxId) -> Result<JobSpec, ValidationErrors> {
        let request = self;
        let timeout = request.timeout_seconds.map_or_else(
            || Ok(JobTimeout::default()),
            |seconds| JobTimeout::try_from(SignedDuration::from_secs(i64::from(seconds))),
        );
        let mut invalid = ValidationErrors::default();
        let mut secrets = BTreeSet::new();
        for (index, name) in request.secrets.into_iter().enumerate() {
            match name.parse::<SecretName>() {
                Ok(name) => {
                    secrets.insert(name);
                }
                Err(error) => invalid.add(index.to_string(), error.to_string()),
            }
        }
        let (argv, env, secrets, timeout) = Validator::new()
            .field("argv", Argv::try_from(request.argv))
            .nested("env", EnvVars::try_from(request.env))
            .nested("secrets", invalid.into_result(secrets))
            .field("timeout_seconds", timeout)
            .finish()?;
        Ok(JobSpec::Execute {
            sandbox,
            argv,
            env,
            secrets,
            timeout,
        })
    }
}

impl From<&Job> for JobResource {
    fn from(job: &Job) -> Self {
        let JobSpec::Execute {
            sandbox,
            argv,
            env,
            secrets,
            timeout,
        } = job.spec();
        let (phase, exit_code, failure_reason) = match job.phase() {
            domain::JobPhase::Queued => (JobPhase::Queued, None, None),
            domain::JobPhase::Leased => (JobPhase::Leased, None, None),
            domain::JobPhase::Running => (JobPhase::Running, None, None),
            domain::JobPhase::Finished { exit_code } => (JobPhase::Finished, Some(exit_code), None),
            domain::JobPhase::Failed { reason } => {
                let reason = match reason {
                    domain::JobFailure::TimedOut => "timed_out",
                    domain::JobFailure::SandboxUnavailable => "sandbox_unavailable",
                    domain::JobFailure::ExecutionError => "execution_error",
                    domain::JobFailure::LeaseLost => "lease_lost",
                };
                (JobPhase::Failed, None, Some(reason.to_owned()))
            }
            domain::JobPhase::Cancelled => (JobPhase::Cancelled, None, None),
        };
        Self {
            id: job.id().to_string(),
            sandbox: sandbox.to_string(),
            argv: argv.clone().into(),
            env: env
                .iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect(),
            secrets: secrets.iter().map(ToString::to_string).collect(),
            timeout_seconds: timeout.get().as_secs().unsigned_abs(),
            phase,
            exit_code,
            failure_reason,
            submitted_at: job.submitted_at(),
        }
    }
}
