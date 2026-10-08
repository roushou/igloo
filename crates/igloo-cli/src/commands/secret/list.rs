//! Lists a repository's secret names.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;

/// Lists a repository's secret names.
#[derive(Args)]
pub(crate) struct List {
    /// The repository id.
    repo: String,
}

impl List {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        for name in client.list_secrets(&self.repo).await?.names {
            println!("{name}");
        }
        Ok(ExitCode::SUCCESS)
    }
}
