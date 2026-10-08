//! Comments on a change.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;
use igloo::change::CommentRequest;

use crate::error::CliError;
use crate::json::Json;

/// Comments on a change's latest revision, or another one, optionally on a line of a file.
#[derive(Args)]
pub(crate) struct Comment {
    /// The change id.
    id: String,
    /// What the comment says.
    body: String,
    /// The revision; the latest when unset.
    #[arg(long)]
    revision: Option<u32>,
    /// The file it is about, relative to the repository root.
    #[arg(long)]
    path: Option<String>,
    /// The line of `--path` it is about.
    #[arg(long, requires = "path")]
    line: Option<u32>,
}

impl Comment {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let mut request = CommentRequest::new(self.body);
        if let Some(revision) = self.revision {
            request = request.on_revision(revision);
        }
        if let Some(path) = self.path {
            request = request.on_path(path, self.line);
        }
        println!(
            "{}",
            Json::from(&client.comment_change(&self.id, &request).await?)
        );
        Ok(ExitCode::SUCCESS)
    }
}
