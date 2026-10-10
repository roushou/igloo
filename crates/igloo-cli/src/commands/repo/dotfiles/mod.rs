//! Manages the dotfiles installed in a repository's new workspaces.

use std::process::ExitCode;

use clap::{Args, Subcommand};
use igloo::Client;

use crate::error::CliError;

mod clear;
mod set;

/// Manages the dotfiles installed once in each new workspace of a repository.
#[derive(Args)]
pub(crate) struct Dotfiles {
    #[command(subcommand)]
    command: DotfilesCommand,
}

#[derive(Subcommand)]
enum DotfilesCommand {
    /// Sets the dotfiles repository and the command that installs it.
    Set(set::Set),
    /// Stops installing dotfiles in new workspaces.
    Clear(clear::Clear),
}

impl Dotfiles {
    /// Executes the selected subcommand through the SDK.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        match self.command {
            DotfilesCommand::Set(command) => command.execute(client).await,
            DotfilesCommand::Clear(command) => command.execute(client).await,
        }
    }
}
