//! Opens a workspace.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;
use igloo::workspace::CreateWorkspaceRequest;

use crate::commands::change::Change;
use crate::error::CliError;
use crate::json::Json;

/// Opens a workspace on a branch of a repository.
#[derive(Args)]
pub(crate) struct Create {
    /// The branch to work on; the repository's default branch when unset.
    #[arg(long)]
    branch: Option<String>,
    /// The repository id; the one matching the `origin` remote when unset.
    #[arg(long)]
    repo: Option<String>,
}

impl Create {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let repo = Change::repo_id(client, self.repo).await?;
        let mut request = CreateWorkspaceRequest::new();
        if let Some(branch) = self.branch {
            request = request.with_branch(branch);
        }
        println!(
            "{}",
            Json::from(&client.create_workspace(&repo, &request).await?)
        );
        Ok(ExitCode::SUCCESS)
    }
}
