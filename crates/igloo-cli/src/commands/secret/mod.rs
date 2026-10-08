//! Manages repository secrets, exposed to jobs that name them.

use std::process::ExitCode;

use clap::{Args, Subcommand};
use igloo::Client;

use crate::error::CliError;

mod delete;
mod list;
mod set;

/// Manages repository secrets, exposed to jobs that name them.
#[derive(Args)]
pub(crate) struct Secret {
    #[command(subcommand)]
    command: SecretCommand,
}

#[derive(Subcommand)]
enum SecretCommand {
    /// Sets a secret to the value read from standard input.
    Set(set::Set),
    /// Lists a repository's secret names.
    List(list::List),
    /// Deletes a secret.
    Delete(delete::Delete),
}

impl Secret {
    /// Executes the selected subcommand through the SDK.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        match self.command {
            SecretCommand::Set(command) => command.execute(client).await,
            SecretCommand::List(command) => command.execute(client).await,
            SecretCommand::Delete(command) => command.execute(client).await,
        }
    }
}
