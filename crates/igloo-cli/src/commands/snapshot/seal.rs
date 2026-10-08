//! Seals a running sandbox into a snapshot and prints the snapshot's id.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;

/// Seals a running sandbox into a snapshot and prints the snapshot's id.
#[derive(Args)]
pub(crate) struct Seal {
    /// The sandbox id.
    sandbox: String,
}

impl Seal {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        println!("{}", client.seal(&self.sandbox).await?);
        Ok(ExitCode::SUCCESS)
    }
}
