//! Manages changes: branches proposed for a repository's default branch.

use std::process::ExitCode;

use clap::{Args, Subcommand};
use igloo::Client;

use crate::error::CliError;
use crate::git::Workspace;

mod approve;
mod checkout;
mod close;
mod comment;
mod create;
mod list;
mod merge;
mod push;
mod request_changes;
mod show;

/// Manages changes: branches proposed for a repository's default branch.
#[derive(Args)]
pub(crate) struct Change {
    #[command(subcommand)]
    command: ChangeCommand,
}

#[derive(Subcommand)]
enum ChangeCommand {
    /// Opens a change for a branch pushed to the forge.
    Create(create::Create),
    /// Records the change's branch head as its next revision, after a push.
    Push(push::Push),
    /// Shows a change, its revisions and the runs of their checks.
    Show(show::Show),
    /// Lists the changes of a repository.
    List(list::List),
    /// Checks out a change's latest revision on its branch.
    Checkout(checkout::Checkout),
    /// Comments on a change's revision, optionally on a line of a file.
    Comment(comment::Comment),
    /// Asks for another revision, carrying the comments since the previous request.
    RequestChanges(request_changes::RequestChanges),
    /// Approves a change's latest revision.
    Approve(approve::Approve),
    /// Merges a change once its checks passed and, for protected paths, a human approved it;
    /// Igloo pushes it to the target branch as one squashed commit.
    Merge(merge::Merge),
    /// Closes a change without merging.
    Close(close::Close),
}

impl Change {
    /// Executes the selected subcommand through the SDK.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        match self.command {
            ChangeCommand::Create(command) => command.execute(client).await,
            ChangeCommand::Push(command) => command.execute(client).await,
            ChangeCommand::Show(command) => command.execute(client).await,
            ChangeCommand::List(command) => command.execute(client).await,
            ChangeCommand::Checkout(command) => command.execute(client).await,
            ChangeCommand::Comment(command) => command.execute(client).await,
            ChangeCommand::RequestChanges(command) => command.execute(client).await,
            ChangeCommand::Approve(command) => command.execute(client).await,
            ChangeCommand::Merge(command) => command.execute(client).await,
            ChangeCommand::Close(command) => command.execute(client).await,
        }
    }

    /// `repo`, or the registered repository whose location is the `origin` remote.
    pub(crate) async fn repo_id(client: &Client, repo: Option<String>) -> Result<String, CliError> {
        if let Some(repo) = repo {
            return Ok(repo);
        }
        let origin = Workspace::current().await?.origin().await?.to_string();
        client
            .list_repos()
            .await?
            .into_iter()
            .find(|repo| repo.location == origin)
            .map(|repo| repo.id)
            .ok_or_else(|| {
                CliError::Usage(format!("no repository registered at {origin}; pass --repo"))
            })
    }
}
