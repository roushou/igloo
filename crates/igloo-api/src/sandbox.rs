//! Sandbox requests and resources.

use std::collections::BTreeMap;

use igloo_core::process::EnvVars;
use igloo_core::repo::RepoId;
use igloo_core::sandbox::{self as domain, ResourceLimits, Sandbox, SandboxSpec};
use igloo_core::snapshot::SnapshotId;
use igloo_core::{Entity, Generation, Labels, Resource, Timestamp, ValidationErrors, Validator};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Creates a sandbox from a snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct CreateSandboxRequest {
    /// The snapshot to start from.
    pub snapshot: String,
    /// CPU and memory bounds; secure defaults when omitted.
    #[serde(default)]
    pub limits: Option<Limits>,
    /// Network access; `deny_all` when omitted.
    #[serde(default)]
    pub network: Option<NetworkPolicy>,
    /// Separation from the host; `any` when omitted.
    #[serde(default)]
    pub isolation: Option<Isolation>,
    /// The repository the sandbox works on (`repo_...`); its secrets become available to jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Environment variables of every process.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Metadata for grouping and filtering.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

/// CPU and memory bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct Limits {
    /// CPU in thousandths of a core, 100 to 64000.
    pub millicpus: u32,
    /// Memory in MiB, 128 to 262144.
    pub memory_mib: u32,
}

/// Network access of a sandbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum NetworkPolicy {
    /// No network access.
    DenyAll,
    /// Unrestricted network access.
    AllowAll,
}

/// How strongly a sandbox is separated from its host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Isolation {
    /// Whatever runtime the placed worker offers, including unisolated development runtimes.
    Any,
    /// A container with its own root file system, namespaces and cgroup limits.
    Container,
}

/// Whether the sandbox should run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DesiredState {
    /// It should run.
    Running,
    /// It should be stopped.
    Stopped,
}

/// Where a sandbox is in its lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SandboxPhase {
    /// Waiting for a worker.
    Pending,
    /// Assigned to a worker.
    Scheduled,
    /// Starting on its worker.
    Starting,
    /// Running.
    Running,
    /// Stopping.
    Stopping,
    /// Stopped.
    Stopped,
    /// Failed; see `failure_reason`.
    Failed,
}

/// A sandbox: its desired spec and observed status.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct SandboxResource {
    /// Its id (`sbx_...`).
    pub id: String,
    /// The snapshot it starts from.
    pub snapshot: String,
    /// Whether it should run.
    pub desired: DesiredState,
    /// CPU and memory bounds.
    pub limits: Limits,
    /// Network access.
    pub network: NetworkPolicy,
    /// Separation from the host.
    pub isolation: Isolation,
    /// The repository it works on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Environment variables.
    pub env: BTreeMap<String, String>,
    /// Labels.
    pub labels: BTreeMap<String, String>,
    /// The observed phase.
    pub phase: SandboxPhase,
    /// Why it failed, when `phase` is `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// The worker it is assigned to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
    /// The spec generation; bumped by every spec change.
    pub generation: u64,
    /// The generation the worker last reported acting on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<u64>,
    /// When it was created.
    #[schema(value_type = String, format = DateTime)]
    pub created_at: Timestamp,
}

/// A page of sandboxes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct SandboxList {
    /// The sandboxes, ordered by id.
    pub items: Vec<SandboxResource>,
    /// Pass as `cursor` to get the next page; absent on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

impl CreateSandboxRequest {
    /// A request for a sandbox from `snapshot` with default limits, no network, no
    /// environment and no labels.
    #[must_use]
    pub fn new(snapshot: impl Into<String>) -> Self {
        Self {
            snapshot: snapshot.into(),
            limits: None,
            network: None,
            isolation: None,
            repo: None,
            env: BTreeMap::new(),
            labels: BTreeMap::new(),
        }
    }

    /// Sets the labels.
    #[must_use]
    pub fn with_labels(mut self, labels: BTreeMap<String, String>) -> Self {
        self.labels = labels;
        self
    }

    /// Sets the environment.
    #[must_use]
    pub fn with_env(mut self, env: BTreeMap<String, String>) -> Self {
        self.env = env;
        self
    }

    /// Sets the network policy.
    #[must_use]
    pub const fn with_network(mut self, network: NetworkPolicy) -> Self {
        self.network = Some(network);
        self
    }

    /// Sets the repository the sandbox works on.
    #[must_use]
    pub fn with_repo(mut self, repo: impl Into<String>) -> Self {
        self.repo = Some(repo.into());
        self
    }

    /// Sets the isolation.
    #[must_use]
    pub const fn with_isolation(mut self, isolation: Isolation) -> Self {
        self.isolation = Some(isolation);
        self
    }
}

impl SandboxList {
    /// A page of `items`, followed by the page after `next_cursor` if any.
    #[must_use]
    pub const fn new(items: Vec<SandboxResource>, next_cursor: Option<String>) -> Self {
        Self { items, next_cursor }
    }
}

impl TryFrom<CreateSandboxRequest> for SandboxSpec {
    type Error = ValidationErrors;

    fn try_from(request: CreateSandboxRequest) -> Result<Self, Self::Error> {
        let limits = request.limits.map_or_else(
            || Ok(ResourceLimits::default()),
            |limits| ResourceLimits::new(limits.millicpus, limits.memory_mib),
        );
        let (snapshot, repo, limits, env, labels) = Validator::new()
            .field("snapshot", request.snapshot.parse::<SnapshotId>())
            .field(
                "repo",
                request
                    .repo
                    .as_deref()
                    .map(str::parse::<RepoId>)
                    .transpose(),
            )
            .nested("limits", limits)
            .nested("env", EnvVars::try_from(request.env))
            .nested("labels", Labels::try_from(request.labels))
            .finish()?;
        Ok(Self::builder()
            .snapshot(snapshot)
            .limits(limits)
            .network(request.network.map_or_else(Default::default, Into::into))
            .isolation(request.isolation.map_or_else(Default::default, Into::into))
            .maybe_repo(repo)
            .env(env)
            .labels(labels)
            .build())
    }
}

impl From<Isolation> for domain::Isolation {
    fn from(isolation: Isolation) -> Self {
        match isolation {
            Isolation::Any => Self::Any,
            Isolation::Container => Self::Container,
        }
    }
}

impl From<domain::Isolation> for Isolation {
    fn from(isolation: domain::Isolation) -> Self {
        match isolation {
            domain::Isolation::Any => Self::Any,
            domain::Isolation::Container => Self::Container,
        }
    }
}

impl From<NetworkPolicy> for domain::NetworkPolicy {
    fn from(policy: NetworkPolicy) -> Self {
        match policy {
            NetworkPolicy::DenyAll => Self::DenyAll,
            NetworkPolicy::AllowAll => Self::AllowAll,
        }
    }
}

impl From<domain::NetworkPolicy> for NetworkPolicy {
    fn from(policy: domain::NetworkPolicy) -> Self {
        match policy {
            domain::NetworkPolicy::DenyAll => Self::DenyAll,
            domain::NetworkPolicy::AllowAll => Self::AllowAll,
        }
    }
}

impl From<domain::DesiredState> for DesiredState {
    fn from(desired: domain::DesiredState) -> Self {
        match desired {
            domain::DesiredState::Running => Self::Running,
            domain::DesiredState::Stopped => Self::Stopped,
        }
    }
}

impl From<domain::SandboxPhase> for SandboxPhase {
    fn from(phase: domain::SandboxPhase) -> Self {
        match phase {
            domain::SandboxPhase::Pending => Self::Pending,
            domain::SandboxPhase::Scheduled => Self::Scheduled,
            domain::SandboxPhase::Starting => Self::Starting,
            domain::SandboxPhase::Running => Self::Running,
            domain::SandboxPhase::Stopping => Self::Stopping,
            domain::SandboxPhase::Stopped => Self::Stopped,
            domain::SandboxPhase::Failed { .. } => Self::Failed,
        }
    }
}

impl From<ResourceLimits> for Limits {
    fn from(limits: ResourceLimits) -> Self {
        Self {
            millicpus: limits.millicpus(),
            memory_mib: limits.memory_mib(),
        }
    }
}

impl From<&Sandbox> for SandboxResource {
    fn from(sandbox: &Sandbox) -> Self {
        let spec = sandbox.spec();
        let status = sandbox.status();
        let failure_reason = match status.phase() {
            domain::SandboxPhase::Failed { reason } => Some(
                match reason {
                    domain::FailureReason::WorkerLost => "worker_lost",
                    domain::FailureReason::SnapshotUnavailable => "snapshot_unavailable",
                    domain::FailureReason::RuntimeError => "runtime_error",
                }
                .to_owned(),
            ),
            _ => None,
        };
        Self {
            id: sandbox.id().to_string(),
            snapshot: spec.snapshot().to_string(),
            desired: spec.desired().into(),
            limits: spec.limits().into(),
            network: spec.network().into(),
            isolation: spec.isolation().into(),
            repo: spec.repo().map(|repo| repo.to_string()),
            env: spec
                .env()
                .iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect(),
            labels: sandbox
                .labels()
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
            phase: status.phase().into(),
            failure_reason,
            worker: status.worker().map(|worker| worker.to_string()),
            generation: sandbox.generation().get(),
            observed_generation: status.observed_generation().map(Generation::get),
            created_at: sandbox.created_at(),
        }
    }
}
