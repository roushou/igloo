//! Snapshots a repository at a branch's head or a commit, with its checkout in
//! `/workspace`.

use std::process::ExitCode;

use clap::Args;
use igloo::Client;
use igloo::repo::RepoSnapshotRequest;

use crate::error::CliError;
use crate::json::Json;

/// Snapshots a repository at a branch's head or a commit, with its checkout in
/// `/workspace`.
#[derive(Args)]
pub(crate) struct Snapshot {
    /// The repository id.
    repo: String,
    /// The branch, fetched from the forge first.
    #[arg(long, conflicts_with = "commit", required_unless_present = "commit")]
    branch: Option<String>,
    /// A commit of the default branch's history.
    #[arg(long)]
    commit: Option<String>,
    /// The snapshot to check out over, such as an imported image.
    #[arg(long)]
    base: Option<String>,
}

impl Snapshot {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let mut request = match (self.branch, self.commit) {
            (Some(branch), _) => RepoSnapshotRequest::branch(branch),
            (None, commit) => RepoSnapshotRequest::commit(commit.unwrap_or_default()),
        };
        if let Some(base) = self.base {
            request = request.with_base(base);
        }
        println!(
            "{}",
            Json::from(&client.snapshot_repo(&self.repo, &request).await?)
        );
        Ok(ExitCode::SUCCESS)
    }
}
