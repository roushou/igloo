//! Clears a repository's dotfiles.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::commands::change::Change;
use crate::error::CliError;

/// Stops installing dotfiles in new workspaces of the repository.
#[derive(Args)]
pub(crate) struct Clear {
    /// The repository id; the one matching the `origin` remote when unset.
    #[arg(long)]
    repo: Option<String>,
}

impl Clear {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let repo = Change::repo_id(client, self.repo).await?;
        client.clear_dotfiles(&repo).await?;
        Ok(ExitCode::SUCCESS)
    }
}
