//! Takes a task over.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;

/// Takes a task over: no turn starts until you hand it back, and `igloo shell <task id>` opens a
/// terminal in its sandbox. A running turn finishes first.
#[derive(Args)]
pub(crate) struct TakeOver {
    /// The task id.
    id: String,
}

impl TakeOver {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        println!("{}", Json::from(&client.take_over_task(&self.id).await?));
        Ok(ExitCode::SUCCESS)
    }
}
