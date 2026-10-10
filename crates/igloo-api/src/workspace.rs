//! Workspaces: a person's long-lived sandbox on a branch of a repository.

use igloo_core::Timestamp;
use igloo_core::workspace::{self as domain, Workspace};
use igloo_core::{Entity as _, Resource as _};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Opens a workspace on a branch of a repository.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct CreateWorkspaceRequest {
    /// The branch to work on; it must exist. The repository's default branch when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// Where a workspace is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WorkspacePhase {
    /// Its sandbox is being prepared or is starting.
    Starting,
    /// Its sandbox runs; open its terminal at `/v1/sandboxes/{sandbox}/terminal`.
    Running,
    /// Its changes are being sealed and its sandbox stopped.
    Stopping,
    /// No sandbox exists; the next start resumes from `snapshot` when it is set.
    Stopped,
}

/// Whether setting up a workspace's sandbox worked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SetupState {
    /// The setup job exited 0.
    Succeeded,
    /// The setup job exited non-zero or did not complete; see `reason`.
    Failed,
}

/// How setting up a workspace's sandbox ended: pointing `origin` at Igloo and installing the
/// repository's dotfiles. A failed setup leaves the workspace usable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct SetupResource {
    /// Whether it worked.
    pub state: SetupState,
    /// The job that ran it (`job_...`); `GET /v1/jobs/{id}/logs` has its output.
    pub job: String,
    /// Why it failed; absent when it succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct WorkspaceResource {
    /// Its id (`wsp_...`).
    pub id: String,
    /// The person it belongs to.
    pub owner: String,
    /// The repository.
    pub repo: String,
    /// The branch it was opened on.
    pub branch: String,
    /// Where it is.
    pub phase: WorkspacePhase,
    /// Its sandbox, while it has one. Its terminal is the sandbox's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
    /// The snapshot its last stop sealed, which holds everything the workspace has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    /// When a person last used it.
    #[schema(value_type = String, format = DateTime)]
    pub last_activity: Timestamp,
    /// How setting up its sandbox ended; absent until the setup job ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<SetupResource>,
    /// When it was created.
    #[schema(value_type = String, format = DateTime)]
    pub created_at: Timestamp,
}

impl CreateWorkspaceRequest {
    /// A request for the repository's default branch.
    #[must_use]
    pub const fn new() -> Self {
        Self { branch: None }
    }

    /// Asks for `branch` instead of the default branch.
    #[must_use]
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }
}

impl From<domain::WorkspacePhase> for WorkspacePhase {
    fn from(phase: domain::WorkspacePhase) -> Self {
        match phase {
            domain::WorkspacePhase::Starting => Self::Starting,
            domain::WorkspacePhase::Running => Self::Running,
            domain::WorkspacePhase::Stopping => Self::Stopping,
            domain::WorkspacePhase::Stopped => Self::Stopped,
        }
    }
}

impl From<&domain::SetupResult> for SetupResource {
    fn from(result: &domain::SetupResult) -> Self {
        let (state, reason) = match &result.outcome {
            domain::SetupOutcome::Succeeded => (SetupState::Succeeded, None),
            domain::SetupOutcome::Failed { reason } => (SetupState::Failed, Some(reason.clone())),
        };
        Self {
            state,
            job: result.job.to_string(),
            reason,
        }
    }
}

impl From<&Workspace> for WorkspaceResource {
    fn from(workspace: &Workspace) -> Self {
        let spec = workspace.spec();
        let status = workspace.status();
        Self {
            id: workspace.id().to_string(),
            owner: spec.owner.to_string(),
            repo: spec.repo.to_string(),
            branch: spec.branch.to_string(),
            phase: status.phase().into(),
            sandbox: status.sandbox().map(|sandbox| sandbox.to_string()),
            snapshot: status.snapshot().map(|snapshot| snapshot.to_string()),
            last_activity: status.last_activity(),
            setup: status.setup().map(SetupResource::from),
            created_at: workspace.created_at(),
        }
    }
}
