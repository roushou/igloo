//! Sets a secret to the value read from standard input.
use std::io::Read;

use std::process::ExitCode;

use clap::Args;
use igloo::Client;

use crate::error::CliError;

/// Sets a secret to the value read from standard input.
#[derive(Args)]
pub(crate) struct Set {
    /// The repository id.
    repo: String,
    /// The secret, such as `GITHUB_TOKEN`.
    name: String,
}

impl Set {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let mut value = String::new();
        std::io::stdin().read_to_string(&mut value)?;
        let value = value.strip_suffix('\n').unwrap_or(&value);
        client.set_secret(&self.repo, &self.name, value).await?;
        Ok(ExitCode::SUCCESS)
    }
}
