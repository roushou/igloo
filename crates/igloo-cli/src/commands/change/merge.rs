//! Merges a change once its checks passed and, for protected paths, a human approved it;
//! Igloo pushes the target branch.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;

/// Merges a change once its checks passed and, for protected paths, a human approved it;
/// Igloo pushes it to the target branch as one squashed commit.
#[derive(Args)]
pub(crate) struct Merge {
    /// The change id.
    id: String,
}

impl Merge {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        println!("{}", Json::from(&client.merge_change(&self.id).await?));
        Ok(ExitCode::SUCCESS)
    }
}
