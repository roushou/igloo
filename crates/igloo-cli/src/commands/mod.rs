//! CLI commands, each owning its arguments and SDK calls.

use std::process::ExitCode;

use clap::Subcommand;
use igloo::Client;

use crate::error::CliError;

pub(crate) mod change;
pub(crate) mod git_credential;
mod logs;
mod repo;
mod run;
mod sandbox;
mod secret;
mod snapshot;
mod task;
mod workspace;

/// The selected CLI command; global connection settings are supplied by the caller.
#[derive(Subcommand)]
pub(crate) enum Command {
    /// Runs a command in a sandbox made from the current directory, streaming its output and
    /// exiting with its exit code.
    Run(run::Run),
    /// Manages changes: branches proposed for a repository's default branch.
    Change(change::Change),
    /// Manages tasks: agents working toward a goal on a repository.
    Task(task::Task),
    /// Manages workspaces: long-lived sandboxes where a person works on a branch.
    Workspace(workspace::Workspace),
    /// Manages repositories.
    Repo(repo::Repo),
    /// Manages repository secrets, exposed to jobs that name them.
    Secret(secret::Secret),
    /// Manages snapshots.
    Snapshot(snapshot::Snapshot),
    /// Manages sandboxes.
    Sandbox(sandbox::Sandbox),
    /// Follows a job's output.
    Logs(logs::Logs),
}

impl Command {
    /// Executes the selected command through the SDK.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        match self {
            Self::Run(cmd) => cmd.execute(client).await,
            Self::Change(cmd) => cmd.execute(client).await,
            Self::Task(cmd) => cmd.execute(client).await,
            Self::Workspace(cmd) => cmd.execute(client).await,
            Self::Repo(cmd) => cmd.execute(client).await,
            Self::Secret(cmd) => cmd.execute(client).await,
            Self::Snapshot(cmd) => cmd.execute(client).await,
            Self::Sandbox(cmd) => cmd.execute(client).await,
            Self::Logs(cmd) => cmd.execute(client).await,
        }
    }
}
