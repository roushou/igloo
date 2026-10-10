use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use igloo_core::repo::{BranchName, Repo, RepoId, SecretName};
use igloo_core::sandbox::SandboxId;
use igloo_core::seal::SealId;
use igloo_core::snapshot::SnapshotId;
use igloo_core::workspace::{Workspace, WorkspaceId};
use igloo_core::{Entity, Resource, ValidationErrors};

use crate::app::{AppError, Command, CommandHandler, RequestContext};
use crate::ports::{
    Clock, EntityStore, IdGenerator, IdGeneratorExt, SecretStore, StorageError, Versioned,
};

/// Opens a workspace on a branch of a repository for the person making the request.
pub struct CreateWorkspace {
    /// The repository.
    pub repo: RepoId,
    /// The branch; it must exist.
    pub branch: BranchName,
}

/// Declares a workspace should run.
pub struct StartWorkspace {
    /// The workspace.
    pub workspace: WorkspaceId,
}

/// Declares a workspace should stop, sealing its changes first.
pub struct StopWorkspace {
    /// The workspace.
    pub workspace: WorkspaceId,
}

/// Stops a workspace if it has been idle for [`Workspace::IDLE_TIMEOUT`].
pub struct StopIdleWorkspace {
    /// The workspace.
    pub workspace: WorkspaceId,
}

/// Deletes a workspace: its sandbox is stopped without sealing and its changes are dropped.
pub struct DeleteWorkspace {
    /// The workspace.
    pub workspace: WorkspaceId,
}

/// Records that a person is using a workspace.
pub struct TouchWorkspace {
    /// The workspace.
    pub workspace: WorkspaceId,
}

/// Records the sandbox opened for a workspace.
pub struct RecordWorkspaceSandbox {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The sandbox.
    pub sandbox: SandboxId,
    /// The repository secrets granted to terminals in it.
    pub secrets: BTreeSet<SecretName>,
}

/// Records that a workspace's sandbox runs.
pub struct RecordWorkspaceRunning {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The sandbox.
    pub sandbox: SandboxId,
}

/// Records the seal of a stopping workspace.
pub struct RecordWorkspaceSealing {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The seal.
    pub seal: SealId,
}

/// Records how a workspace's seal ended.
pub struct RecordWorkspaceSeal {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The seal.
    pub seal: SealId,
    /// The snapshot, or `Err` when the seal failed.
    pub snapshot: Result<SnapshotId, ()>,
}

/// Records that a workspace's sandbox was asked to stop.
pub struct RecordWorkspaceStopping {
    /// The workspace.
    pub workspace: WorkspaceId,
}

/// Records that a workspace's sandbox ended.
pub struct RecordWorkspaceEnded {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The sandbox.
    pub sandbox: SandboxId,
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
    CreateWorkspace => WorkspaceId, "workspace.create";
    StartWorkspace => (), "workspace.start";
    StopWorkspace => (), "workspace.stop";
    StopIdleWorkspace => (), "workspace.stop_idle";
    DeleteWorkspace => (), "workspace.delete";
    TouchWorkspace => (), "workspace.touch";
    RecordWorkspaceSandbox => (), "workspace.record_sandbox";
    RecordWorkspaceRunning => (), "workspace.record_running";
    RecordWorkspaceSealing => (), "workspace.record_sealing";
    RecordWorkspaceSeal => (), "workspace.record_seal";
    RecordWorkspaceStopping => (), "workspace.record_stopping";
    RecordWorkspaceEnded => (), "workspace.record_ended";
}

/// Read access to workspaces beyond loading one by id. Deleted workspaces are never returned,
/// except by the lookups that follow a sandbox or a seal through the end of their stop.
#[derive(Clone)]
pub struct WorkspaceQueries {
    store: Arc<dyn EntityStore<Workspace>>,
}

impl WorkspaceQueries {
    /// Queries over `store`.
    #[must_use]
    pub fn new(store: Arc<dyn EntityStore<Workspace>>) -> Self {
        Self { store }
    }

    /// Workspace `id`, unless it does not exist or was deleted.
    pub async fn get(&self, id: WorkspaceId) -> Result<Option<Workspace>, StorageError> {
        Ok(self
            .store
            .load(id)
            .await?
            .map(Versioned::into_inner)
            .filter(|workspace| !workspace.is_deleted()))
    }

    /// The workspaces that were not deleted, oldest first.
    pub async fn all(&self) -> Result<Vec<Workspace>, StorageError> {
        let mut workspaces: Vec<Workspace> = self
            .store
            .all()
            .await?
            .into_iter()
            .filter(|workspace| !workspace.is_deleted())
            .collect();
        workspaces.sort_by_key(Entity::id);
        Ok(workspaces)
    }

    /// The workspace whose live sandbox is `sandbox`, deleted or not.
    pub async fn with_sandbox(
        &self,
        sandbox: SandboxId,
    ) -> Result<Option<Workspace>, StorageError> {
        Ok(self
            .store
            .all()
            .await?
            .into_iter()
            .find(|workspace| workspace.status().sandbox() == Some(sandbox)))
    }

    /// The workspace being sealed by `seal`, deleted or not.
    pub async fn with_seal(&self, seal: SealId) -> Result<Option<Workspace>, StorageError> {
        Ok(self
            .store
            .all()
            .await?
            .into_iter()
            .find(|workspace| workspace.seal() == Some(seal)))
    }
}

/// Creates a workspace after checking its repository exists and the request has a person.
pub(super) struct CreateWorkspaceHandler {
    pub(super) store: Arc<dyn EntityStore<Workspace>>,
    pub(super) repos: Arc<dyn EntityStore<Repo>>,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) ids: Arc<dyn IdGenerator>,
}

#[async_trait]
impl CommandHandler<CreateWorkspace> for CreateWorkspaceHandler {
    async fn handle(
        &self,
        command: CreateWorkspace,
        context: &RequestContext,
    ) -> Result<WorkspaceId, AppError> {
        let Some(owner) = context.actor.principal() else {
            return Err(
                ValidationErrors::single("owner", "a workspace belongs to a person").into(),
            );
        };
        if self.repos.load(command.repo).await?.is_none() {
            return Err(AppError::not_found(Repo::NAME, &command.repo));
        }
        let id = self.ids.next::<Workspace>();
        let now = self.clock.now();
        let workspace = Workspace::new(id, owner, command.repo, command.branch, now);
        self.store
            .commit(&mut Versioned::new(workspace), &context.commit_meta(now))
            .await?;
        Ok(id)
    }
}

/// The repository secrets a workspace's terminals get as environment variables: those its
/// sandbox was opened with, read from its repository when the terminal opens.
#[derive(Clone)]
pub struct WorkspaceSecrets {
    workspaces: WorkspaceQueries,
    secrets: Arc<dyn SecretStore>,
}

impl WorkspaceSecrets {
    /// Secrets read from `secrets` for the workspaces in `workspaces`.
    #[must_use]
    pub fn new(workspaces: WorkspaceQueries, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            workspaces,
            secrets,
        }
    }

    /// The environment for a terminal in `sandbox`: empty unless the sandbox belongs to a
    /// running workspace. A granted secret that is no longer set is left out.
    pub async fn terminal_env(
        &self,
        sandbox: SandboxId,
    ) -> Result<BTreeMap<String, String>, AppError> {
        let Some(workspace) = self.workspaces.with_sandbox(sandbox).await? else {
            return Ok(BTreeMap::new());
        };
        let mut env = BTreeMap::new();
        for name in workspace.status().secrets() {
            if let Some(value) = self.secrets.get(workspace.spec().repo, name).await? {
                env.insert(name.to_string(), value.expose().to_owned());
            }
        }
        Ok(env)
    }

    /// The workspace whose sandbox is `sandbox`, if any.
    pub async fn workspace_of(&self, sandbox: SandboxId) -> Result<Option<WorkspaceId>, AppError> {
        Ok(self
            .workspaces
            .with_sandbox(sandbox)
            .await?
            .map(|workspace| workspace.id()))
    }
}
