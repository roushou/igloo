//! Stops a workspace, sealing its changes first.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;

/// Stops a workspace, sealing its changes first.
#[derive(Args)]
pub(crate) struct Stop {
    /// The workspace id.
    id: String,
}

impl Stop {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        println!("{}", Json::from(&client.stop_workspace(&self.id).await?));
        Ok(ExitCode::SUCCESS)
    }
}
