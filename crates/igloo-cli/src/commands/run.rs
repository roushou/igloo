//! Runs a command in a sandbox made from the current directory, streaming its output and
//! exiting with its exit code.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;
use igloo::Run as SdkRun;
use igloo::sandbox::Isolation;

use crate::error::CliError;

use super::logs::Logs;
use crate::label::Label;

/// Runs a command in a sandbox made from the current directory, streaming its output and
/// exiting with its exit code.
#[derive(Args)]
pub(crate) struct Run {
    /// Sandbox labels, `key=value`; repeatable.
    #[arg(long = "label")]
    labels: Vec<Label>,
    /// Seconds the command may run.
    #[arg(long)]
    timeout: Option<u32>,
    /// A snapshot to layer the directory over, such as an imported image.
    #[arg(long)]
    base: Option<String>,
    /// Runs in a container: its own file system, network and resource limits. Needs a
    /// worker with the OCI runtime.
    #[arg(long)]
    isolated: bool,
    /// Keeps the sandbox after the command ends.
    #[arg(long)]
    keep: bool,
    /// The command, after `--`.
    #[arg(last = true, required = true)]
    argv: Vec<String>,
}

impl Run {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let mut run =
            SdkRun::new(self.argv).with_labels(self.labels.into_iter().map(Into::into).collect());
        if let Some(seconds) = self.timeout {
            run = run.with_timeout_seconds(seconds);
        }
        if let Some(base) = self.base {
            run = run.with_base(base);
        }
        if self.isolated {
            run = run.with_isolation(Isolation::Container);
        }
        let dir = std::env::current_dir()?;
        let mut running = client.run(&dir, run).await?;
        let end = Logs::follow(running.logs()).await;
        if !self.keep {
            client.stop_sandbox(&running.sandbox.id).await?;
        }
        Ok(Logs::exit_code(end?.as_ref()))
    }
}
