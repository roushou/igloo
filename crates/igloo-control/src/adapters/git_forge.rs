use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use igloo_core::Timestamp;
use igloo_core::repo::{BranchName, CommitId, RepoId};
use igloo_git::{
    ConfigKey, Credentials, DiffStatus, Endpoint, Git, GitError, Layout, Lease, PushRejection,
    RefName, Refspec, RemoteUrl, RepoPath, Repository, Signature,
};

use crate::ports::{Expected, Forge, ForgeError, Remote};

/// Forges reached with the `git` binary in an isolated environment: GitHub over HTTPS with a
/// token, or a repository on the server's file system. Each repository has a bare mirror under
/// `root`; operations that change a mirror run one at a time.
///
/// Invariant: a token is passed to git through its environment, never its arguments.
pub struct GitForge {
    git: Git,
    root: PathBuf,
    locks: Mutex<HashMap<RepoId, Arc<tokio::sync::Mutex<()>>>>,
}

impl GitForge {
    /// The user GitHub expects with a token over HTTPS.
    const TOKEN_USER: &'static str = "x-access-token";
    /// Where an imported bundle's `HEAD` lands in the mirror.
    const IMPORTED: &'static str = "refs/igloo/imported";
    /// Lets shallow checkouts fetch commits by id rather than by branch.
    const ALLOW_ANY_COMMIT: &'static str = "uploadpack.allowAnySHA1InWant";
    /// The committer of the commits the forge creates, such as squashes.
    const COMMITTER: (&'static str, &'static str) = ("Igloo", "igloo@igloo.invalid");

    /// Mirrors under `root`.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            git: Git::isolated(),
            root,
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// The mirror of `repo`.
    #[must_use]
    pub fn mirror(&self, repo: RepoId) -> PathBuf {
        self.root.join(format!("{repo}.git"))
    }

    fn lock(&self, repo: RepoId) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(locks.entry(repo).or_default())
    }

    /// The mirror of `remote`, created if missing.
    async fn ensure_mirror(&self, remote: &Remote) -> Result<Repository, ForgeError> {
        let mirror = self.mirror(remote.repo);
        if tokio::fs::try_exists(mirror.join("HEAD"))
            .await
            .map_err(Self::io)?
        {
            return Ok(self.git.open(mirror).await?);
        }
        let repository = self.git.init(mirror, Layout::Bare, None).await?;
        let key: ConfigKey = Self::ALLOW_ANY_COMMIT.parse().map_err(Self::io)?;
        repository.set_config(&key, "true").await?;
        Ok(repository)
    }

    /// The mirror of `repo`, holding every one of `commits`.
    async fn holding(&self, repo: RepoId, commits: &[&CommitId]) -> Result<Repository, ForgeError> {
        let missing = |commit: &CommitId| ForgeError::CommitNotFound(commit.clone());
        let mirror = match self.git.open(self.mirror(repo)).await {
            Ok(mirror) => mirror,
            Err(GitError::NotARepository(_)) => {
                return Err(commits.first().map_or_else(
                    || ForgeError::Git(format!("no mirror of {repo}")),
                    |commit| missing(commit),
                ));
            }
            Err(error) => return Err(error.into()),
        };
        for commit in commits {
            if !mirror.contains(commit).await? {
                return Err(missing(commit));
            }
        }
        Ok(mirror)
    }

    /// Where `remote` is fetched from and pushed to, with its token.
    fn endpoint(remote: &Remote) -> Endpoint {
        Endpoint::Url {
            url: RemoteUrl::from(&remote.location),
            credentials: remote
                .token
                .as_ref()
                .map(|token| Credentials::basic(Self::TOKEN_USER, token.expose())),
        }
    }

    async fn paths(
        &self,
        repo: RepoId,
        from: &CommitId,
        to: &CommitId,
        only: Option<DiffStatus>,
    ) -> Result<Vec<String>, ForgeError> {
        let mirror = self.holding(repo, &[from, to]).await?;
        Ok(mirror
            .diff(from, to)
            .await?
            .into_iter()
            .filter(|entry| only.is_none_or(|status| entry.status() == status))
            .map(|entry| RepoPath::from(entry).into())
            .collect())
    }

    fn io(error: impl std::fmt::Display) -> ForgeError {
        ForgeError::Git(error.to_string())
    }
}

impl From<GitError> for ForgeError {
    /// git's failure and its causes, as one message.
    fn from(error: GitError) -> Self {
        let mut message = error.to_string();
        let mut source = std::error::Error::source(&error);
        while let Some(cause) = source {
            message = format!("{message}: {cause}");
            source = cause.source();
        }
        Self::Git(message)
    }
}

#[async_trait]
impl Forge for GitForge {
    async fn fetch(&self, remote: &Remote, branch: &BranchName) -> Result<CommitId, ForgeError> {
        let lock = self.lock(remote.repo);
        let _guard = lock.lock().await;
        let mirror = self.ensure_mirror(remote).await?;
        let refspec = Refspec::new(branch).to(branch).forced();
        match mirror
            .fetch(&Self::endpoint(remote), &[refspec], None)
            .await
        {
            Err(GitError::RemoteRefNotFound) => {
                return Err(ForgeError::BranchNotFound(branch.clone()));
            }
            fetched => fetched?,
        }
        mirror
            .resolve(&RefName::from(branch))
            .await?
            .ok_or_else(|| ForgeError::BranchNotFound(branch.clone()))
    }

    async fn mirrored(
        &self,
        repo: RepoId,
        branch: &BranchName,
    ) -> Result<Option<CommitId>, ForgeError> {
        match self.git.open(self.mirror(repo)).await {
            Ok(mirror) => Ok(mirror.resolve(&RefName::from(branch)).await?),
            Err(GitError::NotARepository(_)) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    async fn checkout(
        &self,
        repo: RepoId,
        commit: &CommitId,
        into: &Path,
    ) -> Result<(), ForgeError> {
        let lock = self.lock(repo);
        let _guard = lock.lock().await;
        let mirror = self.holding(repo, &[commit]).await?;
        let source: RemoteUrl = format!("file://{}", mirror.path().display())
            .parse()
            .map_err(Self::io)?;
        let tree = self.git.init(into, Layout::WorkTree, None).await?;
        tree.fetch(
            &Endpoint::url(source),
            &[Refspec::new(commit.clone())],
            Some(NonZeroU32::MIN),
        )
        .await?;
        tree.checkout_detached(commit).await?;
        Ok(())
    }

    async fn import(&self, repo: RepoId, bundle: &Path) -> Result<CommitId, ForgeError> {
        let lock = self.lock(repo);
        let _guard = lock.lock().await;
        let mirror = self.holding(repo, &[]).await?;
        mirror.verify_bundle(bundle).await?;
        let imported: RefName = Self::IMPORTED.parse().map_err(Self::io)?;
        let refspec = Refspec::new(RefName::head()).to(imported.clone()).forced();
        mirror
            .fetch(&Endpoint::Bundle(bundle.to_owned()), &[refspec], None)
            .await?;
        mirror
            .resolve(&imported)
            .await?
            .ok_or_else(|| ForgeError::Git("the bundle has no HEAD commit".to_owned()))
    }

    async fn read_file(
        &self,
        repo: RepoId,
        commit: &CommitId,
        path: &str,
    ) -> Result<Option<Vec<u8>>, ForgeError> {
        let mirror = self.holding(repo, &[commit]).await?;
        let Ok(path) = path.parse::<RepoPath>() else {
            return Ok(None);
        };
        Ok(mirror.read_blob(commit, &path).await?)
    }

    async fn squash(
        &self,
        repo: RepoId,
        onto: &CommitId,
        head: &CommitId,
        message: &str,
        at: Timestamp,
    ) -> Result<CommitId, ForgeError> {
        let mirror = self.holding(repo, &[onto, head]).await?;
        let oldest = mirror
            .commits(onto, head)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| ForgeError::Git(format!("{head} adds no commit to {onto}")))?;
        let seconds = jiff::Timestamp::from(at).as_second();
        let author = mirror.author(oldest.id()).await?.at(seconds);
        let (name, email) = Self::COMMITTER;
        let committer = Signature::new(name, email, seconds);
        Ok(mirror
            .commit_tree(head, onto, message, &author, &committer)
            .await?)
    }

    async fn merge_base(
        &self,
        repo: RepoId,
        a: &CommitId,
        b: &CommitId,
    ) -> Result<Option<CommitId>, ForgeError> {
        let mirror = self.holding(repo, &[a, b]).await?;
        Ok(mirror.merge_base(a, b).await?)
    }

    async fn commits(
        &self,
        repo: RepoId,
        from: &CommitId,
        to: &CommitId,
    ) -> Result<Vec<(CommitId, String)>, ForgeError> {
        let mirror = self.holding(repo, &[from, to]).await?;
        Ok(mirror
            .commits(from, to)
            .await?
            .into_iter()
            .map(|commit| (commit.id().clone(), commit.message().to_owned()))
            .collect())
    }

    async fn changed_paths(
        &self,
        repo: RepoId,
        from: &CommitId,
        to: &CommitId,
    ) -> Result<Vec<String>, ForgeError> {
        self.paths(repo, from, to, None).await
    }

    async fn deleted_paths(
        &self,
        repo: RepoId,
        from: &CommitId,
        to: &CommitId,
    ) -> Result<Vec<String>, ForgeError> {
        self.paths(repo, from, to, Some(DiffStatus::Deleted)).await
    }

    async fn push(
        &self,
        remote: &Remote,
        commit: &CommitId,
        branch: &BranchName,
        expected: Expected,
    ) -> Result<(), ForgeError> {
        let lock = self.lock(remote.repo);
        let _guard = lock.lock().await;
        let mirror = self.ensure_mirror(remote).await?;
        if !mirror.contains(commit).await? {
            return Err(ForgeError::CommitNotFound(commit.clone()));
        }
        let lease = match expected {
            Expected::Any => Lease::Any,
            Expected::Absent => Lease::Absent,
            Expected::At(current) => Lease::At(current),
        };
        match mirror
            .push(&Self::endpoint(remote), commit, branch, &lease)
            .await
        {
            Ok(()) => Ok(()),
            Err(GitError::Rejected(PushRejection::Stale | PushRejection::NotFastForward)) => {
                Err(ForgeError::Moved(branch.clone()))
            }
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::conformance::ForgeConformance;

    #[tokio::test]
    async fn passes_the_forge_conformance_suite() {
        let dir = tempfile::tempdir().expect("dir");
        ForgeConformance::new(
            Arc::new(GitForge::new(dir.path().join("mirrors"))),
            dir.path(),
        )
        .run_all()
        .await;
    }
}
