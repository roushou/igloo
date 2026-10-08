//! Manages sandboxes.

use std::process::ExitCode;

use clap::{Args, Subcommand};
use igloo::Client;

use crate::error::CliError;

mod create;
mod get;
mod list;
mod stop;

/// Manages sandboxes.
#[derive(Args)]
pub(crate) struct Sandbox {
    #[command(subcommand)]
    command: SandboxCommand,
}

#[derive(Subcommand)]
enum SandboxCommand {
    /// Creates a sandbox from a snapshot.
    Create(create::Create),
    /// Shows a sandbox.
    Get(get::Get),
    /// Lists sandboxes, optionally filtered by labels.
    List(list::List),
    /// Stops a sandbox.
    Stop(stop::Stop),
}

impl Sandbox {
    /// Executes the selected subcommand through the SDK.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        match self.command {
            SandboxCommand::Create(command) => command.execute(client).await,
            SandboxCommand::Get(command) => command.execute(client).await,
            SandboxCommand::List(command) => command.execute(client).await,
            SandboxCommand::Stop(command) => command.execute(client).await,
        }
    }
}
