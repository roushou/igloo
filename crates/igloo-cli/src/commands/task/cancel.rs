//! Cancels a task.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;

/// Cancels a task and stops its sandbox.
#[derive(Args)]
pub(crate) struct Cancel {
    /// The task id.
    id: String,
}

impl Cancel {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        println!("{}", Json::from(&client.cancel_task(&self.id).await?));
        Ok(ExitCode::SUCCESS)
    }
}
