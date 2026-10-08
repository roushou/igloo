use serde::{Deserialize, Serialize};

use crate::process::EnvVars;
use crate::repo::RepoId;
use crate::snapshot::SnapshotId;
use crate::{Labels, ValidationErrors};

/// The desired state of a sandbox, declared by clients.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, bon::Builder)]
pub struct SandboxSpec {
    snapshot: SnapshotId,
    #[builder(default)]
    desired: DesiredState,
    #[builder(default)]
    limits: ResourceLimits,
    #[builder(default)]
    network: NetworkPolicy,
    #[builder(default)]
    #[serde(default)]
    isolation: Isolation,
    /// The repository the sandbox works on; its secrets are available to its jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    repo: Option<RepoId>,
    #[builder(default)]
    env: EnvVars,
    #[builder(default)]
    labels: Labels,
}

/// Whether the sandbox should run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredState {
    /// The sandbox should be running.
    #[default]
    Running,
    /// The sandbox should be stopped. Final.
    Stopped,
}

/// Network access of a sandbox. Denied unless explicitly allowed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    /// No network access.
    #[default]
    DenyAll,
    /// Unrestricted network access.
    AllowAll,
}

/// How strongly a sandbox is separated from its host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Isolation {
    /// Any runtime the placed worker offers, including unisolated development runtimes.
    #[default]
    Any,
    /// A container: its own root file system, process and network namespaces, and cgroup
    /// limits. Only workers offering the OCI runtime host it.
    Container,
}

/// CPU and memory bounds of a sandbox.
///
/// Invariant: `millicpus` in `100..=64000`, `memory_mib` in `128..=262144`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawLimits", into = "RawLimits")]
pub struct ResourceLimits {
    millicpus: u32,
    memory_mib: u32,
}

#[derive(Serialize, Deserialize)]
struct RawLimits {
    millicpus: u32,
    memory_mib: u32,
}

impl SandboxSpec {
    /// The snapshot the sandbox starts from.
    #[must_use]
    pub const fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }

    /// Whether the sandbox should run.
    #[must_use]
    pub const fn desired(&self) -> DesiredState {
        self.desired
    }

    /// CPU and memory bounds.
    #[must_use]
    pub const fn limits(&self) -> ResourceLimits {
        self.limits
    }

    /// Network access.
    #[must_use]
    pub const fn network(&self) -> NetworkPolicy {
        self.network
    }

    /// The repository the sandbox works on, if any.
    #[must_use]
    pub const fn repo(&self) -> Option<RepoId> {
        self.repo
    }

    /// How strongly the sandbox is separated from its host.
    #[must_use]
    pub const fn isolation(&self) -> Isolation {
        self.isolation
    }

    /// Environment variables of every process in the sandbox.
    #[must_use]
    pub const fn env(&self) -> &EnvVars {
        &self.env
    }

    /// Metadata for grouping and filtering.
    #[must_use]
    pub const fn labels(&self) -> &Labels {
        &self.labels
    }

    pub(super) fn stop(&mut self) {
        self.desired = DesiredState::Stopped;
    }
}

impl ResourceLimits {
    const MILLICPUS: std::ops::RangeInclusive<u32> = 100..=64_000;
    const MEMORY_MIB: std::ops::RangeInclusive<u32> = 128..=262_144;

    /// Limits within bounds, reporting every field out of range.
    pub fn new(millicpus: u32, memory_mib: u32) -> Result<Self, ValidationErrors> {
        let mut errors = ValidationErrors::default();
        if !Self::MILLICPUS.contains(&millicpus) {
            errors.add("millicpus", format!("must be in {:?}", Self::MILLICPUS));
        }
        if !Self::MEMORY_MIB.contains(&memory_mib) {
            errors.add("memory_mib", format!("must be in {:?}", Self::MEMORY_MIB));
        }
        errors.into_result(Self {
            millicpus,
            memory_mib,
        })
    }

    /// CPU in thousandths of a core.
    #[must_use]
    pub const fn millicpus(&self) -> u32 {
        self.millicpus
    }

    /// Memory in MiB.
    #[must_use]
    pub const fn memory_mib(&self) -> u32 {
        self.memory_mib
    }
}

impl Default for ResourceLimits {
    /// One core and 2 GiB.
    fn default() -> Self {
        Self {
            millicpus: 1000,
            memory_mib: 2048,
        }
    }
}

impl TryFrom<RawLimits> for ResourceLimits {
    type Error = ValidationErrors;

    fn try_from(raw: RawLimits) -> Result<Self, Self::Error> {
        Self::new(raw.millicpus, raw.memory_mib)
    }
}

impl From<ResourceLimits> for RawLimits {
    fn from(limits: ResourceLimits) -> Self {
        Self {
            millicpus: limits.millicpus,
            memory_mib: limits.memory_mib,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_report_every_field_out_of_range() {
        let errors = ResourceLimits::new(1, 1).expect_err("both out of range");
        assert_eq!(errors.fields().count(), 2);
        assert!(ResourceLimits::new(100, 128).is_ok());
    }
}
