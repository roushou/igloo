//! Shows a task and its turns.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;

/// Shows a task and its turns; `igloo logs <job>` follows a turn's output.
#[derive(Args)]
pub(crate) struct Show {
    /// The task id.
    id: String,
}

impl Show {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        println!("{}", Json::from(&client.get_task(&self.id).await?));
        Ok(ExitCode::SUCCESS)
    }
}
