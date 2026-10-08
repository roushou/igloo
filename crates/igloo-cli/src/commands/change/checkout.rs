//! Checks out a change's latest revision on its branch.

use std::process::ExitCode;

use clap::Args;
use igloo::{Client, Error};
use igloo_core::repo::{BranchName, CommitId};

use crate::error::CliError;
use crate::git::Workspace;

/// Checks out a change's latest revision on its branch.
#[derive(Args)]
pub(crate) struct Checkout {
    /// The change id.
    id: String,
}

impl Checkout {
    /// Executes the command through the SDK and returns its process exit code.
    pub(crate) async fn execute(self, client: &Client) -> Result<ExitCode, CliError> {
        let change = client.get_change(&self.id).await?;
        let invalid = |what: &str| Error::Protocol(format!("change {} has {what}", self.id));
        let branch: BranchName = change
            .source_branch
            .parse()
            .map_err(|_| invalid("an invalid branch"))?;
        let head: CommitId = change
            .revisions
            .last()
            .and_then(|revision| revision.head.parse().ok())
            .ok_or_else(|| invalid("no valid revision"))?;
        Workspace::current().await?.checkout(&branch, &head).await?;
        println!("{branch} at {head}");
        Ok(ExitCode::SUCCESS)
    }
}
