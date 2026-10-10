//! Opens a terminal in a workspace from this machine's terminal.

use std::process::ExitCode;
use std::time::Duration;

use clap::Args;
use igloo::workspace::WorkspacePhase;
use igloo::{Client, Terminal, TerminalEndReason, TerminalEvent, TerminalExit};
use tokio::signal::unix::{SignalKind, signal};

use self::console::Console;
use crate::commands::change::Change;
use crate::error::CliError;

mod console;

/// Opens a terminal in a workspace, starting it first when it is stopped, or, given a task id, in
/// the sandbox of a task you took over (`igloo task take-over`). The local terminal is put in raw
/// mode and follows resizes; the command exits with the remote shell's exit code.
#[derive(Args)]
pub(crate) struct Shell {
    /// The workspace id, or the id of a task you took over; your most recently used workspace of
    /// the repository when unset.
    workspace: Option<String>,
    /// The repository id for choosing the workspace when none is named; the one matching the
    /// `origin` remote of the current directory when unset.
    #[arg(long)]
    repo: Option<String>,
}

impl Shell {
    /// How long a workspace may take to run before the shell gives up on it.
    const START_TIMEOUT: Duration = Duration::from_mins(10);
    /// What the id of a task starts with.
    const TASK_PREFIX: &'static str = "task_";
    /// The exit code when the terminal ended without the remote shell's: the connection was
    /// lost or the process could not run, as `ssh` reports its own failures.
    const LOST: u8 = 255;

    /// Executes the command through the SDK and returns the remote shell's exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let sandbox = match self.workspace.as_deref() {
            Some(id) if id.starts_with(Self::TASK_PREFIX) => Self::task_sandbox(client, id).await?,
            _ => self.workspace_sandbox(client).await?,
        };

        let exit = {
            let mut console = Console::attach()?;
            let (cols, rows) = Console::size();
            let terminal = client.open_terminal(&sandbox, &[], cols, rows).await?;
            Self::attached(&mut console, terminal).await?
        };
        Ok(match exit {
            Some(TerminalExit::Code(code)) => ExitCode::from(Self::status(code)),
            Some(TerminalExit::Failed(reason)) => {
                eprintln!("igloo: the terminal ended: {}", Self::describe(reason));
                ExitCode::from(Self::LOST)
            }
            Some(_) | None => {
                eprintln!("igloo: the connection to the terminal was lost");
                ExitCode::from(Self::LOST)
            }
        })
    }

    /// The sandbox of the workspace to open, which is started first when it is stopped.
    async fn workspace_sandbox(&self, client: &Client) -> Result<String, CliError> {
        let id = self.resolve(client).await?;
        let workspace = client.get_workspace(&id).await?;
        if workspace.phase != WorkspacePhase::Running {
            eprintln!("igloo: workspace {id} is not running; starting it");
        }
        let workspace = client
            .start_workspace_and_wait(&id, Self::START_TIMEOUT)
            .await?;
        workspace.sandbox.ok_or_else(|| {
            CliError::Usage(format!(
                "workspace {id} has no sandbox to open a terminal in"
            ))
        })
    }

    /// The sandbox of task `id`, which must be taken over: the server lets only the person who
    /// took it over type in it.
    async fn task_sandbox(client: &Client, id: &str) -> Result<String, CliError> {
        let task = client.get_task(id).await?;
        if task.takeover.is_none() {
            return Err(CliError::Usage(format!(
                "task {id} is not taken over; take it over first with `igloo task take-over {id}`"
            )));
        }
        task.sandbox.ok_or_else(|| {
            CliError::Usage(format!("task {id} has no sandbox to open a terminal in"))
        })
    }

    /// The workspace to open: the one named, or the latest of the repository.
    async fn resolve(&self, client: &Client) -> Result<String, CliError> {
        if let Some(workspace) = &self.workspace {
            return Ok(workspace.clone());
        }
        let repo = Change::repo_id(client, self.repo.clone()).await?;
        client
            .latest_workspace(&repo)
            .await?
            .map(|workspace| workspace.id)
            .ok_or_else(|| {
                CliError::Usage(format!(
                    "you have no workspace of {repo}; open one with `igloo workspace create`"
                ))
            })
    }

    /// Carries the console to and from `terminal` until the remote process ends: `None` when
    /// the connection ended without an exit.
    async fn attached(
        console: &mut Console,
        terminal: Terminal,
    ) -> Result<Option<TerminalExit>, CliError> {
        let (mut input, mut output) = terminal.split();
        let mut typing = true;
        let mut resizes = signal(SignalKind::window_change())?;
        let mut hangups = signal(SignalKind::hangup())?;
        let mut terminations = signal(SignalKind::terminate())?;
        loop {
            tokio::select! {
                event = output.next() => match event? {
                    Some(TerminalEvent::Output(bytes)) => Console::print(&bytes)?,
                    Some(TerminalEvent::Exit(exit)) => return Ok(Some(exit)),
                    Some(_) => {}
                    None => return Ok(None),
                },
                typed = console.typed(), if typing => {
                    if let Some(bytes) = typed {
                        input.send(bytes).await?;
                    } else {
                        typing = false;
                        if !console.is_interactive() {
                            // End of piped input: the remote shell reads end-of-file.
                            input.send(vec![Self::END_OF_INPUT]).await?;
                        }
                    }
                }
                _ = resizes.recv() => {
                    let (cols, rows) = Console::size();
                    input.resize(cols, rows).await?;
                }
                _ = hangups.recv() => return Ok(Some(TerminalExit::Code(129))),
                _ = terminations.recv() => return Ok(Some(TerminalExit::Code(143))),
            }
        }
    }

    /// Ctrl-D.
    const END_OF_INPUT: u8 = 0x04;

    /// The process status for a remote exit code: the code modulo 256, as a shell reports it.
    fn status(code: i32) -> u8 {
        u8::try_from(code.rem_euclid(256)).unwrap_or(Self::LOST)
    }

    fn describe(reason: TerminalEndReason) -> &'static str {
        match reason {
            TerminalEndReason::SandboxUnavailable => "the sandbox is not running on its worker",
            TerminalEndReason::ExecutionError => "the shell could not be started",
            TerminalEndReason::Closed => "the terminal was closed",
            TerminalEndReason::Lost => "the worker's connection dropped",
            _ => "unknown reason",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_exit_code_becomes_the_process_status() {
        assert_eq!(Shell::status(0), 0);
        assert_eq!(Shell::status(7), 7);
        assert_eq!(Shell::status(130), 130);
        assert_eq!(Shell::status(256), 0);
        assert_eq!(Shell::status(-1), 255);
    }
}
