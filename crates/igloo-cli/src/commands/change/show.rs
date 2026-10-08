//! Shows a change, its revisions and the runs of their checks.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;

/// Shows a change, its revisions and the runs of their checks.
#[derive(Args)]
pub(crate) struct Show {
    /// The change id.
    id: String,
}

impl Show {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let change = client.get_change(&self.id).await?;
        let runs = client.list_runs(&self.id).await?;
        println!(
            "{}",
            Json::from(&serde_json::json!({ "change": change, "runs": runs }))
        );
        Ok(ExitCode::SUCCESS)
    }
}
