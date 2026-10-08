//! Follows a job's output.
use std::io::Write;

use std::process::ExitCode;

use clap::Args;
use igloo::job::JobPhase;
use igloo::{Client, Error};

use crate::error::CliError;
use igloo::{LogEvent, OutputStream};

/// Follows a job's output.
#[derive(Args)]
pub(crate) struct Logs {
    /// The job id.
    job: String,
}

impl Logs {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let mut logs = client.logs(&self.job).await?;
        let end = Self::follow(&mut logs).await?;
        Ok(Self::exit_code(end.as_ref()))
    }

    /// Prints output as it arrives; returns the finished job.
    pub(crate) async fn follow(
        logs: &mut igloo::LogStream,
    ) -> Result<Option<igloo::job::JobResource>, Error> {
        while let Some(event) = logs.next().await {
            match event? {
                LogEvent::Output {
                    stream: OutputStream::Stderr,
                    data,
                } => {
                    eprint!("{data}");
                    let _ = std::io::stderr().flush();
                }
                LogEvent::Output { data, .. } => {
                    print!("{data}");
                    let _ = std::io::stdout().flush();
                }
                LogEvent::End(job) => return Ok(Some(*job)),
                _ => {}
            }
        }
        Ok(None)
    }

    /// The job's exit code; 1 when it did not finish normally.
    pub(crate) fn exit_code(job: Option<&igloo::job::JobResource>) -> ExitCode {
        match job {
            Some(job) if job.phase == JobPhase::Finished => {
                ExitCode::from(u8::try_from(job.exit_code.unwrap_or(1)).unwrap_or(1))
            }
            _ => ExitCode::FAILURE,
        }
    }
}
