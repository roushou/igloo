//! Prints a task's transcript.

use std::process::ExitCode;
use std::time::Duration;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;

/// Prints a task's transcript, one JSON entry per entry: what its tool said and did.
#[derive(Args)]
pub(crate) struct Transcript {
    /// The task id.
    id: String,
    /// Keeps printing new entries until no turn runs.
    #[arg(long)]
    follow: bool,
}

impl Transcript {
    /// How often a followed transcript is read again.
    const POLL: Duration = Duration::from_secs(1);

    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let mut after = 0;
        loop {
            let transcript = client.get_transcript(&self.id, after).await?;
            for entry in &transcript.entries {
                println!("{}", Json::from(entry));
            }
            after = transcript.next;
            if !self.follow || transcript.idle {
                return Ok(ExitCode::SUCCESS);
            }
            tokio::time::sleep(Self::POLL).await;
        }
    }
}
