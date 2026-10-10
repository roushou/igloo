//! Starts a stopped workspace, resuming from what its last stop sealed.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;

/// Starts a stopped workspace, resuming from what its last stop sealed.
#[derive(Args)]
pub(crate) struct Start {
    /// The workspace id.
    id: String,
}

impl Start {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        println!("{}", Json::from(&client.start_workspace(&self.id).await?));
        Ok(ExitCode::SUCCESS)
    }
}
