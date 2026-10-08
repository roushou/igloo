//! Registers a repository.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;
use igloo::repo::RegisterRepoRequest;

use crate::error::CliError;
use crate::json::Json;

/// Registers a repository.
#[derive(Args)]
pub(crate) struct Add {
    /// `github.com/<owner>/<name>`, or an absolute path on the server.
    location: String,
    /// The branch changes merge into; `main` when unset.
    #[arg(long)]
    default_branch: Option<String>,
    /// The repository secret holding the forge token, such as `GITHUB_TOKEN`.
    #[arg(long)]
    token_secret: Option<String>,
}

impl Add {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let mut request = RegisterRepoRequest::new(self.location);
        if let Some(branch) = self.default_branch {
            request = request.with_default_branch(branch);
        }
        if let Some(name) = self.token_secret {
            request = request.with_token_secret(name);
        }
        println!("{}", Json::from(&client.register_repo(&request).await?));
        Ok(ExitCode::SUCCESS)
    }
}
