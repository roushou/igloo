//! Lists the changes of a repository.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;

use super::Change;
use crate::json::Json;

/// Lists the changes of a repository.
#[derive(Args)]
pub(crate) struct List {
    /// The repository id; the one matching the `origin` remote when unset.
    #[arg(long)]
    repo: Option<String>,
}

impl List {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let repo = Change::repo_id(client, self.repo).await?;
        println!("{}", Json::from(&client.list_changes(&repo).await?));
        Ok(ExitCode::SUCCESS)
    }
}
