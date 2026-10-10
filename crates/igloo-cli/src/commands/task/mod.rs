//! Manages tasks: agents working toward a goal on a repository.

use std::process::ExitCode;

use clap::{Args, Subcommand};
use igloo::Client;

use crate::error::CliError;

mod cancel;
mod create;
mod hand_back;
mod list;
mod show;
mod take_over;
mod transcript;

/// Manages tasks: agents working toward a goal on a repository.
#[derive(Args)]
pub(crate) struct Task {
    #[command(subcommand)]
    command: TaskCommand,
}

#[derive(Subcommand)]
enum TaskCommand {
    /// Creates a task; the agent starts from the head of the repository's default branch.
    Create(create::Create),
    /// Shows a task and its turns.
    Show(show::Show),
    /// Lists the tasks of a repository.
    List(list::List),
    /// Prints a task's transcript: what its tool said and did.
    Transcript(transcript::Transcript),
    /// Cancels a task and stops its sandbox.
    Cancel(cancel::Cancel),
    /// Takes a task over: its agent starts no turn and you can type in its sandbox.
    TakeOver(take_over::TakeOver),
    /// Hands a taken-over task back to its agent, telling it what you changed.
    HandBack(hand_back::HandBack),
}

impl Task {
    /// Executes the selected subcommand through the SDK.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        match self.command {
            TaskCommand::Create(command) => command.execute(client).await,
            TaskCommand::Show(command) => command.execute(client).await,
            TaskCommand::List(command) => command.execute(client).await,
            TaskCommand::Transcript(command) => command.execute(client).await,
            TaskCommand::Cancel(command) => command.execute(client).await,
            TaskCommand::TakeOver(command) => command.execute(client).await,
            TaskCommand::HandBack(command) => command.execute(client).await,
        }
    }
}
