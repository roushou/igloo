//! Creates a task.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;
use igloo::task::CreateTaskRequest;

use crate::commands::change::Change;
use crate::error::CliError;
use crate::json::Json;

/// Creates a task; the agent starts from the head of the repository's default branch.
#[derive(Args)]
pub(crate) struct Create {
    /// What the agent is asked to do.
    goal: String,
    /// A tool of the repository's `.igloo/agents.toml`; its default tool when unset.
    #[arg(long)]
    tool: Option<String>,
    /// The repository id; the one matching the `origin` remote when unset.
    #[arg(long)]
    repo: Option<String>,
}

impl Create {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let repo = Change::repo_id(client, self.repo).await?;
        let mut request = CreateTaskRequest::new(self.goal);
        if let Some(tool) = self.tool {
            request = request.with_tool(tool);
        }
        println!(
            "{}",
            Json::from(&client.create_task(&repo, &request).await?)
        );
        Ok(ExitCode::SUCCESS)
    }
}
