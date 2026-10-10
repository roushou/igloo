//! Workspaces: a person's long-lived sandbox on a branch, resumed from what its last stop
//! sealed. A product over the platform's sandboxes, seals and checkouts, as agents are.

mod commands;
mod lifecycle;
mod reactors;
mod setup;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use igloo_core::job::Job;
use igloo_core::repo::Repo;
use igloo_core::seal::Seal;
use igloo_core::workspace::{Workspace, WorkspaceError};
use reqwest::Url;

use self::commands::CreateWorkspaceHandler;
pub use self::commands::{
    CreateWorkspace, DeleteWorkspace, RecordWorkspaceEnded, RecordWorkspaceRunning,
    RecordWorkspaceSandbox, RecordWorkspaceSeal, RecordWorkspaceSealing, RecordWorkspaceStopping,
    StartWorkspace, StopIdleWorkspace, StopWorkspace, TouchWorkspace, WorkspaceQueries,
    WorkspaceSecrets,
};
use self::lifecycle::Lifecycle;
use self::reactors::{FollowSandboxes, FollowSeals};
use self::setup::{ReportSetups, SetUpWorkspaces, WorkspaceSetup};
use crate::app::{ControllerSettings, EntityHandler, Extension, InstallError, PlatformBuilder};
use crate::ci::CiModule;
use crate::platform::{RepoQueries, RepoSnapshots, SandboxQueries};

/// Workspaces: their commands, controller, and the reactors feeding it sandbox and seal
/// outcomes.
pub struct WorkspaceModule {
    /// Pacing of the workspace controller.
    pub settings: ControllerSettings,
    /// The public URL of the server, under which each repository's git is served at
    /// `/git/<repository id>.git`: the `origin` of every new workspace's checkout.
    pub git_base: Url,
}

impl Extension for WorkspaceModule {
    fn name(&self) -> &'static str {
        "workspaces"
    }

    fn install(self: Box<Self>, platform: &mut PlatformBuilder) -> Result<(), InstallError> {
        let store = platform.store::<Workspace>()?;
        Self::commands(platform, &store)?;
        let ports = platform.ports().clone();
        let lifecycle = Lifecycle {
            bus: platform.bus(),
            ids: Arc::clone(&ports.ids),
            repos: RepoQueries::new(platform.store::<Repo>()?, Arc::clone(&ports.secrets)),
            forge: Arc::clone(&ports.forge),
            checkouts: RepoSnapshots::new(Arc::clone(&ports.forge), Arc::clone(&ports.blobs)),
            environments: CiModule::environments(platform),
            sandboxes: SandboxQueries::new(platform.store()?),
            seals: platform.store::<Seal>()?,
        };
        platform.controller(lifecycle, self.settings)?;
        let workspaces = WorkspaceQueries::new(store);
        platform.reactor(FollowSandboxes {
            workspaces: workspaces.clone(),
        });
        platform.reactor(FollowSeals {
            workspaces: workspaces.clone(),
        });
        platform.reactor(SetUpWorkspaces {
            setup: WorkspaceSetup::new(self.git_base),
            workspaces,
            repos: platform.store::<Repo>()?,
        });
        platform.reactor(ReportSetups {
            jobs: platform.store::<Job>()?,
        });
        Ok(())
    }
}

impl WorkspaceModule {
    fn commands(
        platform: &mut PlatformBuilder,
        store: &Arc<dyn crate::ports::EntityStore<Workspace>>,
    ) -> Result<(), InstallError> {
        let ports = platform.ports().clone();
        let clock = &ports.clock;
        platform.command(CreateWorkspaceHandler {
            store: Arc::clone(store),
            repos: platform.store::<Repo>()?,
            clock: Arc::clone(clock),
            ids: Arc::clone(&ports.ids),
        })?;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &StartWorkspace| command.workspace,
            |workspace: &mut Workspace, _: &StartWorkspace, now| -> Result<(), WorkspaceError> {
                workspace.start(now)
            },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &StopWorkspace| command.workspace,
            |workspace: &mut Workspace, _: &StopWorkspace, _| workspace.stop(),
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &StopIdleWorkspace| command.workspace,
            |workspace: &mut Workspace, _: &StopIdleWorkspace, now| workspace.stop_if_idle(now),
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &DeleteWorkspace| command.workspace,
            |workspace: &mut Workspace, _: &DeleteWorkspace, now| workspace.delete(now),
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &TouchWorkspace| command.workspace,
            |workspace: &mut Workspace, _: &TouchWorkspace, now| workspace.touch(now),
        ))?;
        Self::record_commands(platform, store)
    }

    /// The commands recording each step of a workspace's opening and stopping.
    fn record_commands(
        platform: &mut PlatformBuilder,
        store: &Arc<dyn crate::ports::EntityStore<Workspace>>,
    ) -> Result<(), InstallError> {
        let clock = Arc::clone(&platform.ports().clock);
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(&clock),
            |command: &RecordWorkspaceSandbox| command.workspace,
            |workspace: &mut Workspace,
             command: &RecordWorkspaceSandbox,
             _|
             -> Result<(), WorkspaceError> {
                workspace.sandbox_created(command.sandbox, command.secrets.clone())
            },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(&clock),
            |command: &RecordWorkspaceRunning| command.workspace,
            |workspace: &mut Workspace, command: &RecordWorkspaceRunning, now| {
                workspace.sandbox_running(command.sandbox, now);
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(&clock),
            |command: &RecordWorkspaceSealing| command.workspace,
            |workspace: &mut Workspace,
             command: &RecordWorkspaceSealing,
             _|
             -> Result<(), WorkspaceError> { workspace.sealing(command.seal) },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(&clock),
            |command: &RecordWorkspaceSeal| command.workspace,
            |workspace: &mut Workspace, command: &RecordWorkspaceSeal, _| {
                workspace.seal_ended(command.seal, command.snapshot);
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(&clock),
            |command: &RecordWorkspaceStopping| command.workspace,
            |workspace: &mut Workspace,
             _: &RecordWorkspaceStopping,
             _|
             -> Result<(), WorkspaceError> { workspace.sandbox_stopping() },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            clock,
            |command: &RecordWorkspaceEnded| command.workspace,
            |workspace: &mut Workspace, command: &RecordWorkspaceEnded, _| {
                workspace.sandbox_ended(command.sandbox);
            },
        ))?;
        Ok(())
    }
}
