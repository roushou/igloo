//! Lists the caller's workspaces.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;

/// Lists your workspaces.
#[derive(Args)]
pub(crate) struct List;

impl List {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        println!("{}", Json::from(&client.list_workspaces().await?));
        Ok(ExitCode::SUCCESS)
    }
}
