//! Asks for another revision of a change.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;

/// Asks for another revision, carrying the comments since the previous request; a task's
/// agent gets them as its next turn.
#[derive(Args)]
pub(crate) struct RequestChanges {
    /// The change id.
    id: String,
}

impl RequestChanges {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        println!("{}", Json::from(&client.request_changes(&self.id).await?));
        Ok(ExitCode::SUCCESS)
    }
}
