//! Opens a change for a branch pushed to the forge.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;
use igloo::change::OpenChangeRequest;

use crate::error::CliError;

use super::Change;
use crate::git::Workspace;
use crate::json::Json;

/// Opens a change for a branch pushed to the forge.
#[derive(Args)]
pub(crate) struct Create {
    /// The branch; the current one when unset.
    #[arg(long)]
    branch: Option<String>,
    /// The title; the latest commit's subject when unset.
    #[arg(long)]
    title: Option<String>,
    /// The repository id; the one matching the `origin` remote when unset.
    #[arg(long)]
    repo: Option<String>,
}

impl Create {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let repo = Change::repo_id(client, self.repo).await?;
        let (branch, title) = match (self.branch, self.title) {
            (Some(branch), Some(title)) => (branch, title),
            (branch, title) => {
                let workspace = Workspace::current().await?;
                let branch = match branch {
                    Some(branch) => branch,
                    None => workspace.branch().await?.into(),
                };
                let title = match title {
                    Some(title) => title,
                    None => workspace.last_subject().await?,
                };
                (branch, title)
            }
        };
        let request = OpenChangeRequest::new(branch, title);
        println!(
            "{}",
            Json::from(&client.open_change(&repo, &request).await?)
        );
        Ok(ExitCode::SUCCESS)
    }
}
