//! Sets a repository's dotfiles.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;
use igloo::repo::DotfilesResource;

use crate::commands::change::Change;
use crate::error::CliError;
use crate::json::Json;

/// Sets the dotfiles repository cloned into each new workspace and the shell command run in the
/// clone to install it. A failing install is reported and does not fail the workspace.
#[derive(Args)]
pub(crate) struct Set {
    /// The URL to clone: `https://`, `http://`, `ssh://` or `git@`.
    repository: String,
    /// The shell command run in the clone, such as `./install.sh`.
    install: String,
    /// The repository id; the one matching the `origin` remote when unset.
    #[arg(long)]
    repo: Option<String>,
}

impl Set {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let repo = Change::repo_id(client, self.repo).await?;
        let dotfiles = DotfilesResource::new(self.repository, self.install);
        println!(
            "{}",
            Json::from(&client.set_dotfiles(&repo, &dotfiles).await?)
        );
        Ok(ExitCode::SUCCESS)
    }
}
