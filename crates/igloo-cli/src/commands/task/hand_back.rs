//! Hands a taken-over task back.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;

/// Hands a task you took over back to its agent. Its next turn is told which commits you added
/// and what you left uncommitted; uncommitted work stays in the sandbox. Allowed once the turn
/// that was running has ended.
#[derive(Args)]
pub(crate) struct HandBack {
    /// The task id.
    id: String,
}

impl HandBack {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        println!("{}", Json::from(&client.hand_back_task(&self.id).await?));
        Ok(ExitCode::SUCCESS)
    }
}
