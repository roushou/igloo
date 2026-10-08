//! The local git repository the CLI runs in.

use std::path::PathBuf;

use igloo_core::repo::{BranchName, CommitId, RepoLocation};
use igloo_git::{Endpoint, Git, Head, Refspec, RemoteName, Repository};

use crate::error::CliError;

/// The repository of the current directory, driven with the person's git configuration.
pub(crate) struct Workspace {
    repository: Repository,
}

impl Workspace {
    /// The repository containing the current directory.
    pub(crate) async fn current() -> Result<Self, CliError> {
        Self::at(".").await
    }

    /// The repository containing `dir`.
    async fn at(dir: impl Into<PathBuf>) -> Result<Self, CliError> {
        let repository = Git::user().open(dir).await?;
        Ok(Self { repository })
    }

    /// The checked-out branch; refused when `HEAD` is detached.
    pub(crate) async fn branch(&self) -> Result<BranchName, CliError> {
        match self.repository.head().await? {
            Head::Branch { name, .. } => Ok(name),
            Head::Detached(_) => Err(CliError::Usage(
                "HEAD is detached; check out a branch or pass --branch".to_owned(),
            )),
        }
    }

    /// The subject of the checked-out commit; refused before the first commit.
    pub(crate) async fn last_subject(&self) -> Result<String, CliError> {
        let head = self.repository.head().await?;
        let commit = head.commit().ok_or_else(|| {
            CliError::Usage("the branch has no commits yet; pass --title".to_owned())
        })?;
        Ok(self.repository.commit(commit).await?.subject().to_owned())
    }

    /// Where the `origin` remote is, as Igloo names repositories.
    pub(crate) async fn origin(&self) -> Result<RepoLocation, CliError> {
        let url = self.repository.remote_url(&RemoteName::origin()).await?;
        RepoLocation::try_from(&url).map_err(|_| {
            CliError::Usage(format!(
                "origin ({url}) is not a GitHub repository or a local path; pass --repo"
            ))
        })
    }

    /// Fetches `branch` from `origin` and checks out `commit` on it, creating or resetting the
    /// local branch.
    pub(crate) async fn checkout(
        &self,
        branch: &BranchName,
        commit: &CommitId,
    ) -> Result<(), CliError> {
        let origin = Endpoint::Remote(RemoteName::origin());
        self.repository
            .fetch(&origin, &[Refspec::new(branch)], None)
            .await?;
        self.repository.switch(branch, commit).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use igloo_git::testing::Fixture;

    use super::*;

    #[tokio::test]
    async fn the_workspace_reads_its_branch_subject_and_origin() {
        let dir = tempfile::tempdir().expect("dir");
        let fixture = Fixture::new(dir.path());
        let workspace = Workspace::at(fixture.work()).await.expect("open");
        assert!(matches!(
            workspace.last_subject().await,
            Err(CliError::Usage(_))
        ));
        let first = fixture.commit("Add a file\n\nWith a body", &[("a", Some("a"))]);
        assert_eq!(workspace.branch().await.expect("branch").as_str(), "main");
        assert_eq!(
            workspace.last_subject().await.expect("subject"),
            "Add a file"
        );
        fixture.git(&["checkout", "--quiet", "--detach"]);
        assert!(matches!(workspace.branch().await, Err(CliError::Usage(_))));

        assert!(matches!(workspace.origin().await, Err(CliError::Git(_))));
        let origin = fixture.origin().display().to_string();
        fixture.git(&["remote", "add", "origin", &origin]);
        assert_eq!(
            workspace.origin().await.expect("origin"),
            fixture.location()
        );
        fixture.git(&[
            "remote",
            "set-url",
            "origin",
            "git@github.com:roushou/igloo.git",
        ]);
        assert_eq!(
            workspace.origin().await.expect("origin").to_string(),
            "github.com/roushou/igloo"
        );
        fixture.git(&["remote", "set-url", "origin", "https://gitlab.com/o/n.git"]);
        assert!(matches!(workspace.origin().await, Err(CliError::Usage(_))));

        fixture.git(&["remote", "set-url", "origin", &origin]);
        fixture.switch("feature");
        let second = fixture.commit("Second", &[("b", Some("b"))]);
        fixture.git(&["checkout", "--quiet", "--detach", first.as_str()]);
        fixture.git(&["branch", "--quiet", "--delete", "--force", "feature"]);
        let feature: BranchName = "feature".parse().expect("branch");
        workspace
            .checkout(&feature, &second)
            .await
            .expect("checkout");
        assert_eq!(workspace.branch().await.expect("branch"), feature);
        assert_eq!(workspace.last_subject().await.expect("subject"), "Second");
    }
}
