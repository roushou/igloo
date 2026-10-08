//! Deletes a secret.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;

/// Deletes a secret.
#[derive(Args)]
pub(crate) struct Delete {
    /// The repository id.
    repo: String,
    /// The secret.
    name: String,
}

impl Delete {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        client.delete_secret(&self.repo, &self.name).await?;
        Ok(ExitCode::SUCCESS)
    }
}
