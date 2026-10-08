//! Approves a change's latest revision.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;

/// Approves a change's latest revision.
#[derive(Args)]
pub(crate) struct Approve {
    /// The change id.
    id: String,
}

impl Approve {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        println!("{}", Json::from(&client.approve_change(&self.id).await?));
        Ok(ExitCode::SUCCESS)
    }
}
