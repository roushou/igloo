//! Stops a sandbox.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;

/// Stops a sandbox.
#[derive(Args)]
pub(crate) struct Stop {
    /// The sandbox id.
    id: String,
}

impl Stop {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        println!("{}", Json::from(&client.stop_sandbox(&self.id).await?));
        Ok(ExitCode::SUCCESS)
    }
}
