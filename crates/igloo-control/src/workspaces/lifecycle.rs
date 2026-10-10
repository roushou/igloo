use std::collections::BTreeSet;
use std::sync::Arc;

use igloo_core::repo::{CommitId, Repo, RepoId};
use igloo_core::sandbox::{NetworkPolicy, SandboxId, SandboxPhase, SandboxSpec};
use igloo_core::seal::{Seal, SealPhase};
use igloo_core::snapshot::SnapshotId;
use igloo_core::workspace::{Workspace, WorkspaceAction, WorkspaceError, WorkspaceId};
use igloo_core::{Actor, Entity, ErrorCode, Resource, SystemComponent};

use super::commands::{
    RecordWorkspaceEnded, RecordWorkspaceRunning, RecordWorkspaceSandbox, RecordWorkspaceSeal,
    RecordWorkspaceSealing, RecordWorkspaceStopping, StopIdleWorkspace,
};
use crate::app::{AppError, Command, CommandBus, Reconciler, RequestContext};
use crate::ci::{Environment, Environments, SandboxSettings};
use crate::platform::{
    CreateSandbox, CreateSeal, RecordWarmUse, RepoQueries, RepoSnapshots, SandboxQueries,
    StopSandbox, WarmRecipe,
};
use crate::ports::{EntityStore, Forge, IdGenerator, IdGeneratorExt};

/// Carries out workspaces' plans: checks out and starts their sandbox, seals it and stops it.
pub(super) struct Lifecycle {
    pub(super) bus: CommandBus,
    pub(super) ids: Arc<dyn IdGenerator>,
    pub(super) repos: RepoQueries,
    pub(super) forge: Arc<dyn Forge>,
    pub(super) checkouts: RepoSnapshots,
    pub(super) environments: Environments,
    pub(super) sandboxes: SandboxQueries,
    pub(super) seals: Arc<dyn EntityStore<Seal>>,
}

impl Reconciler<Workspace> for Lifecycle {
    async fn reconcile(
        &self,
        workspace: &Workspace,
        actions: Vec<WorkspaceAction>,
    ) -> Result<(), AppError> {
        for action in actions {
            match action {
                WorkspaceAction::Open => self.open(workspace).await?,
                WorkspaceAction::Seal(sandbox) => self.seal(workspace.id(), sandbox).await?,
                WorkspaceAction::StopSandbox(sandbox) => {
                    self.stop(workspace.id(), sandbox).await?;
                }
                WorkspaceAction::StopIdle => {
                    self.dispatch(StopIdleWorkspace {
                        workspace: workspace.id(),
                    })
                    .await?;
                }
            }
        }
        Ok(())
    }
}

impl Lifecycle {
    /// Starts the workspace's sandbox over the snapshot its last stop sealed, or over the
    /// repository's warm snapshot with the branch head checked out when it has none.
    async fn open(&self, workspace: &Workspace) -> Result<(), AppError> {
        let spec = workspace.spec();
        let repo = self
            .repos
            .get(spec.repo)
            .await?
            .ok_or_else(|| AppError::not_found(Repo::NAME, &spec.repo))?;
        let resumed = workspace.status().snapshot();
        let head = self.head(&repo, workspace, resumed.is_some()).await?;
        let environment = match self.environments.at(&repo, &head).await? {
            Ok(environment) => Some(environment),
            Err(reason) => {
                tracing::warn!(workspace = %workspace.id(), %reason, "opening without a pipeline");
                None
            }
        };
        let snapshot = match resumed {
            Some(snapshot) => snapshot,
            None => self.fresh(&repo, &head, environment.as_ref()).await?,
        };
        let settings = environment
            .as_ref()
            .map(|environment| &environment.pipeline.sandbox);
        let secrets = settings.map_or_else(BTreeSet::new, |settings| settings.secrets.clone());
        let sandbox = self
            .dispatch(CreateSandbox {
                spec: Self::sandbox_spec(repo.id(), snapshot, settings),
            })
            .await?;
        let recorded = self
            .dispatch(RecordWorkspaceSandbox {
                workspace: workspace.id(),
                sandbox,
                secrets,
            })
            .await;
        match recorded {
            Ok(()) => self.catch_up(workspace.id(), sandbox).await,
            // The workspace stopped while its sandbox was being created.
            Err(AppError::Domain { code, .. }) if code == WorkspaceError::OutOfOrder.code() => {
                self.dispatch(StopSandbox { sandbox }).await
            }
            Err(error) => Err(error),
        }
    }

    /// The commit the workspace opens at: the branch's head on the forge, or, when resuming,
    /// the mirror's, which needs no forge.
    async fn head(
        &self,
        repo: &Repo,
        workspace: &Workspace,
        resuming: bool,
    ) -> Result<CommitId, AppError> {
        let branch = &workspace.spec().branch;
        if resuming && let Some(head) = self.forge.mirrored(repo.id(), branch).await? {
            return Ok(head);
        }
        let remote = self.repos.remote(repo).await?;
        Ok(self.forge.fetch(&remote, branch).await?)
    }

    /// The branch head checked out over the repository's warm snapshot for it, or over its
    /// base when no warm snapshot is built.
    async fn fresh(
        &self,
        repo: &Repo,
        head: &CommitId,
        environment: Option<&Environment>,
    ) -> Result<SnapshotId, AppError> {
        let base = environment.and_then(|environment| environment.base);
        let mut warm = None;
        if let Some(recipe) = environment
            .and_then(|environment| environment.pipeline.warm.as_ref())
            .map(|warm| WarmRecipe {
                command: warm.command.clone(),
                lockfiles: warm.lockfiles.clone(),
            })
        {
            let key = self
                .checkouts
                .warm_key(repo.id(), head, base, &recipe)
                .await?;
            if let Some(built) = repo.warm(&key) {
                self.dispatch(RecordWarmUse {
                    repo: repo.id(),
                    keys: vec![key],
                })
                .await?;
                warm = Some(built.clone());
            }
        }
        let base = if warm.is_some() { None } else { base };
        self.checkouts
            .checkout(repo.id(), head, base, warm.as_ref())
            .await
    }

    async fn seal(&self, workspace: WorkspaceId, sandbox: SandboxId) -> Result<(), AppError> {
        let seal = self.dispatch(CreateSeal { sandbox }).await?;
        self.dispatch(RecordWorkspaceSealing { workspace, seal })
            .await?;
        let ended = match self.seals.load(seal).await? {
            Some(seal) => match seal.entity().phase() {
                SealPhase::Sealed { snapshot } => Some(Ok(snapshot)),
                SealPhase::Failed { .. } => Some(Err(())),
                SealPhase::Pending => None,
            },
            None => None,
        };
        match ended {
            Some(snapshot) => {
                self.dispatch(RecordWorkspaceSeal {
                    workspace,
                    seal,
                    snapshot,
                })
                .await
            }
            None => Ok(()),
        }
    }

    async fn stop(&self, workspace: WorkspaceId, sandbox: SandboxId) -> Result<(), AppError> {
        self.dispatch(StopSandbox { sandbox }).await?;
        self.dispatch(RecordWorkspaceStopping { workspace }).await?;
        self.catch_up(workspace, sandbox).await
    }

    /// Records what the sandbox did before the workspace knew it: its events may have gone by
    /// while the workspace was still creating or stopping it.
    async fn catch_up(&self, workspace: WorkspaceId, sandbox: SandboxId) -> Result<(), AppError> {
        let Some(current) = self.sandboxes.get(sandbox).await? else {
            return Ok(());
        };
        match current.status().phase() {
            SandboxPhase::Running => {
                self.dispatch(RecordWorkspaceRunning { workspace, sandbox })
                    .await
            }
            phase if phase.is_terminal() => {
                self.dispatch(RecordWorkspaceEnded { workspace, sandbox })
                    .await
            }
            _ => Ok(()),
        }
    }

    /// A sandbox for a person's work: network allowed, over the pipeline's isolation, limits
    /// and environment when the repository has one.
    fn sandbox_spec(
        repo: RepoId,
        snapshot: SnapshotId,
        settings: Option<&SandboxSettings>,
    ) -> SandboxSpec {
        match settings {
            Some(settings) => SandboxSpec::builder()
                .snapshot(snapshot)
                .repo(repo)
                .isolation(settings.isolation)
                .limits(settings.limits)
                .network(NetworkPolicy::AllowAll)
                .env(settings.env.clone())
                .build(),
            None => SandboxSpec::builder()
                .snapshot(snapshot)
                .repo(repo)
                .network(NetworkPolicy::AllowAll)
                .build(),
        }
    }

    async fn dispatch<C: Command>(&self, command: C) -> Result<C::Output, AppError> {
        let context = RequestContext::new(
            Actor::System {
                component: SystemComponent::Controller,
            },
            self.ids.next(),
        );
        self.bus.dispatch(command, context).await
    }
}
