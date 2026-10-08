//! Closes a change without merging.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;

/// Closes a change without merging.
#[derive(Args)]
pub(crate) struct Close {
    /// The change id.
    id: String,
}

impl Close {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        println!("{}", Json::from(&client.close_change(&self.id).await?));
        Ok(ExitCode::SUCCESS)
    }
}
