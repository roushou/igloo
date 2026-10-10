//! Deletes a workspace.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;

/// Deletes a workspace, dropping its unsealed changes.
#[derive(Args)]
pub(crate) struct Delete {
    /// The workspace id.
    id: String,
}

impl Delete {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        client.delete_workspace(&self.id).await?;
        Ok(ExitCode::SUCCESS)
    }
}
