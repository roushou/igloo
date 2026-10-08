//! Manages snapshots.

use std::process::ExitCode;

use clap::{Args, Subcommand};
use igloo::Client;

use crate::error::CliError;

mod import;
mod seal;

/// Manages snapshots.
#[derive(Args)]
pub(crate) struct Snapshot {
    #[command(subcommand)]
    command: SnapshotCommand,
}

#[derive(Subcommand)]
enum SnapshotCommand {
    /// Seals a running sandbox into a snapshot and prints the snapshot's id.
    Seal(seal::Seal),
    /// Has the server pull a public container image and register it as a snapshot.
    Import(import::Import),
}

impl Snapshot {
    /// Executes the selected subcommand through the SDK.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        match self.command {
            SnapshotCommand::Seal(command) => command.execute(client).await,
            SnapshotCommand::Import(command) => command.execute(client).await,
        }
    }
}
