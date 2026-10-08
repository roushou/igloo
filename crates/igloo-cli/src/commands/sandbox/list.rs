//! Lists sandboxes, optionally filtered by labels.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;
use crate::json::Json;
use crate::label::Label;

/// Lists sandboxes, optionally filtered by labels.
#[derive(Args)]
pub(crate) struct List {
    /// Labels, `key=value`; repeatable.
    #[arg(long = "label")]
    labels: Vec<Label>,
}

impl List {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let labels: Vec<(String, String)> = self.labels.into_iter().map(Into::into).collect();
        println!(
            "{}",
            Json::from(&client.list_sandboxes(&labels, None).await?)
        );
        Ok(ExitCode::SUCCESS)
    }
}
