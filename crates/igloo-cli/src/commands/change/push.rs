//! Records the change's branch head as its next revision, after a push.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;

/// Records the change's branch head as its next revision, after a push.
#[derive(Args)]
pub(crate) struct Push {
    /// The change id.
    id: String,
}

impl Push {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        println!("{}", Json::from(&client.revise_change(&self.id).await?));
        Ok(ExitCode::SUCCESS)
    }
}
