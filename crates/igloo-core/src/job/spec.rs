use jiff::SignedDuration;
use serde::{Deserialize, Serialize};

use std::collections::BTreeSet;

use crate::process::{Argv, EnvVars};
use crate::repo::SecretName;
use crate::sandbox::SandboxId;

/// What a job does. A closed set: product variety lives in the steps inside `Execute`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobSpec {
    /// Run a process in an existing sandbox.
    Execute {
        /// Where to run.
        sandbox: SandboxId,
        /// What to run.
        argv: Argv,
        /// Extra environment, layered over the sandbox's.
        env: EnvVars,
        /// Secrets of the sandbox's repository exposed as environment variables, by name.
        #[serde(default)]
        secrets: BTreeSet<SecretName>,
        /// How long the process may run.
        timeout: JobTimeout,
    },
}

impl JobSpec {
    /// The sandbox the job runs in.
    #[must_use]
    pub const fn sandbox(&self) -> SandboxId {
        match self {
            Self::Execute { sandbox, .. } => *sandbox,
        }
    }

    /// The repository secrets the job is given.
    #[must_use]
    pub const fn secrets(&self) -> &BTreeSet<SecretName> {
        match self {
            Self::Execute { secrets, .. } => secrets,
        }
    }
}

/// How long a job may run.
///
/// Invariant: between one second and 24 hours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SignedDuration", into = "SignedDuration")]
pub struct JobTimeout(SignedDuration);

/// A timeout outside the allowed range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("timeout must be between 1 second and 24 hours")]
pub struct TimeoutOutOfRange;

impl JobTimeout {
    const MIN: SignedDuration = SignedDuration::from_secs(1);
    const MAX: SignedDuration = SignedDuration::from_hours(24);

    /// The duration.
    #[must_use]
    pub const fn get(self) -> SignedDuration {
        self.0
    }
}

impl TryFrom<SignedDuration> for JobTimeout {
    type Error = TimeoutOutOfRange;

    fn try_from(duration: SignedDuration) -> Result<Self, Self::Error> {
        if (Self::MIN..=Self::MAX).contains(&duration) {
            Ok(Self(duration))
        } else {
            Err(TimeoutOutOfRange)
        }
    }
}

impl From<JobTimeout> for SignedDuration {
    fn from(timeout: JobTimeout) -> Self {
        timeout.0
    }
}

impl Default for JobTimeout {
    /// One hour.
    fn default() -> Self {
        Self(SignedDuration::from_hours(1))
    }
}
