//! Manages workspaces: long-lived sandboxes where a person works on a branch.

use std::process::ExitCode;

use clap::{Args, Subcommand};
use igloo::Client;

use crate::error::CliError;

mod create;
mod delete;
mod list;
mod start;
mod stop;

/// Manages workspaces: long-lived sandboxes where a person works on a branch.
#[derive(Args)]
pub(crate) struct Workspace {
    #[command(subcommand)]
    command: WorkspaceCommand,
}

#[derive(Subcommand)]
enum WorkspaceCommand {
    /// Opens a workspace on a branch of a repository.
    Create(create::Create),
    /// Lists your workspaces.
    List(list::List),
    /// Starts a stopped workspace, resuming from what its last stop sealed.
    Start(start::Start),
    /// Stops a workspace, sealing its changes first.
    Stop(stop::Stop),
    /// Deletes a workspace, dropping its unsealed changes.
    Delete(delete::Delete),
}

impl Workspace {
    /// Executes the selected subcommand through the SDK.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        match self.command {
            WorkspaceCommand::Create(command) => command.execute(client).await,
            WorkspaceCommand::List(command) => command.execute(client).await,
            WorkspaceCommand::Start(command) => command.execute(client).await,
            WorkspaceCommand::Stop(command) => command.execute(client).await,
            WorkspaceCommand::Delete(command) => command.execute(client).await,
        }
    }
}
