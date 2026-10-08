//! Manages repositories.

use std::process::ExitCode;

use clap::{Args, Subcommand};
use igloo::Client;

use crate::error::CliError;

mod add;
mod list;
mod snapshot;

/// Manages repositories.
#[derive(Args)]
pub(crate) struct Repo {
    #[command(subcommand)]
    command: RepoCommand,
}

#[derive(Subcommand)]
enum RepoCommand {
    /// Registers a repository.
    Add(add::Add),
    /// Lists repositories.
    List(list::List),
    /// Snapshots a repository at a branch's head or a commit, with its checkout in
    /// `/workspace`.
    Snapshot(snapshot::Snapshot),
}

impl Repo {
    /// Executes the selected subcommand through the SDK.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        match self.command {
            RepoCommand::Add(command) => command.execute(client).await,
            RepoCommand::List(command) => command.execute(client).await,
            RepoCommand::Snapshot(command) => command.execute(client).await,
        }
    }
}
