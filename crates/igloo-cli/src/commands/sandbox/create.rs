//! Creates a sandbox from a snapshot.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;
use igloo::sandbox::CreateSandboxRequest;

use crate::error::CliError;
use crate::json::Json;
use crate::label::Label;

/// Creates a sandbox from a snapshot.
#[derive(Args)]
pub(crate) struct Create {
    /// The snapshot id.
    snapshot: String,
    /// Labels, `key=value`; repeatable.
    #[arg(long = "label")]
    labels: Vec<Label>,
}

impl Create {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let request = CreateSandboxRequest::new(self.snapshot)
            .with_labels(self.labels.into_iter().map(Into::into).collect());
        println!("{}", Json::from(&client.create_sandbox(&request).await?));
        Ok(ExitCode::SUCCESS)
    }
}
