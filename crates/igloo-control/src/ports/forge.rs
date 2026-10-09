use std::path::Path;

use async_trait::async_trait;
use igloo_core::Timestamp;
use igloo_core::repo::{BranchName, CommitId, RepoId, RepoLocation};

use super::SecretValue;

/// The git host of repositories. Igloo keeps a mirror of each repository; fetches land in it
/// and pushes come from it.
#[async_trait]
pub trait Forge: Send + Sync {
    /// Fetches `branch` from the forge into the mirror; returns its head.
    async fn fetch(&self, remote: &Remote, branch: &BranchName) -> Result<CommitId, ForgeError>;

    /// The head of `branch` in `repo`'s mirror as of its last fetch, without reaching the
    /// forge; `None` when the mirror does not exist or lacks the branch.
    async fn mirrored(
        &self,
        repo: RepoId,
        branch: &BranchName,
    ) -> Result<Option<CommitId>, ForgeError>;

    /// Pushes `commit`, present in the mirror, to `branch` on the forge, provided the branch
    /// is as `expected`. A branch already at `commit` is left as it is, whatever `expected`
    /// says, so retrying a push is safe.
    async fn push(
        &self,
        remote: &Remote,
        commit: &CommitId,
        branch: &BranchName,
        expected: Expected,
    ) -> Result<(), ForgeError>;

    /// Deletes `branch` on the forge, provided it is at `at`. A branch already gone counts as
    /// deleted; one at another commit is left as it is and reported as moved.
    async fn delete(
        &self,
        remote: &Remote,
        branch: &BranchName,
        at: &CommitId,
    ) -> Result<(), ForgeError>;

    /// Fetches the commits of the git bundle at `bundle` into `repo`'s mirror; returns the
    /// bundle's `HEAD`. The bundle's prerequisite commits must be in the mirror.
    async fn import(&self, repo: RepoId, bundle: &Path) -> Result<CommitId, ForgeError>;

    /// Writes a working tree of `commit` from `repo`'s mirror into the empty directory
    /// `into`, with a shallow `.git` holding just that commit, checked out detached.
    async fn checkout(
        &self,
        repo: RepoId,
        commit: &CommitId,
        into: &Path,
    ) -> Result<(), ForgeError>;

    /// The content of `path` at `commit` in `repo`'s mirror, if the file exists there.
    async fn read_file(
        &self,
        repo: RepoId,
        commit: &CommitId,
        path: &str,
    ) -> Result<Option<Vec<u8>>, ForgeError>;

    /// Creates in `repo`'s mirror one commit with `head`'s tree on the single parent `onto`,
    /// with `message`, authored by the author of the oldest commit between `onto` and `head`
    /// and committed by Igloo, both dated `at`; returns it. `head` must descend from `onto`.
    /// Equal inputs give the same commit, so a squash can be recomputed to recognize it.
    async fn squash(
        &self,
        repo: RepoId,
        onto: &CommitId,
        head: &CommitId,
        message: &str,
        at: Timestamp,
    ) -> Result<CommitId, ForgeError>;

    /// The best common ancestor of `a` and `b` in `repo`'s mirror; `None` when their histories
    /// are unrelated.
    async fn merge_base(
        &self,
        repo: RepoId,
        a: &CommitId,
        b: &CommitId,
    ) -> Result<Option<CommitId>, ForgeError>;

    /// The commits reachable from `to` and not from `from`, oldest first, with their messages.
    async fn commits(
        &self,
        repo: RepoId,
        from: &CommitId,
        to: &CommitId,
    ) -> Result<Vec<(CommitId, String)>, ForgeError>;

    /// The paths added, modified or deleted between `from` and `to`, sorted; a rename counts
    /// as both its paths.
    async fn changed_paths(
        &self,
        repo: RepoId,
        from: &CommitId,
        to: &CommitId,
    ) -> Result<Vec<String>, ForgeError>;

    /// The files that differ between `from` and `to` in `repo`'s mirror, sorted by path, with
    /// renames detected and the unified patch of each.
    async fn diff(
        &self,
        repo: RepoId,
        from: &CommitId,
        to: &CommitId,
    ) -> Result<Vec<ChangedFile>, ForgeError>;

    /// The paths present at `from` and absent at `to`, sorted.
    async fn deleted_paths(
        &self,
        repo: RepoId,
        from: &CommitId,
        to: &CommitId,
    ) -> Result<Vec<String>, ForgeError>;
}

/// One file that differs between two commits.
///
/// Invariants: `previous_path` is set exactly when `status` is [`FileStatus::Renamed`]; a binary
/// file has no counts and an empty `patch`; any other `patch` is the unified patch exactly as
/// git prints it, from its `diff --git` header on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFile {
    /// The path after the change; for a deletion, the path the file had.
    pub path: String,
    /// The path a renamed file had before.
    pub previous_path: Option<String>,
    /// How the file differs.
    pub status: FileStatus,
    /// Lines added.
    pub additions: u64,
    /// Lines removed.
    pub deletions: u64,
    /// Whether the file is binary.
    pub binary: bool,
    /// The unified patch.
    pub patch: String,
}

/// How a file differs between two commits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    /// Absent before, present after.
    Added,
    /// Present in both with different content, mode or kind.
    Modified,
    /// Present before, absent after.
    Deleted,
    /// Moved from another path, possibly with changes.
    Renamed,
}

/// A repository on its forge, with what is needed to reach it.
#[derive(Clone, Debug)]
pub struct Remote {
    /// The repository; its mirror is keyed by it.
    pub repo: RepoId,
    /// Where it is hosted.
    pub location: RepoLocation,
    /// The forge token, when the forge needs one.
    pub token: Option<SecretValue>,
}

/// What a push expects of the branch it updates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expected {
    /// Anything: the branch is overwritten.
    Any,
    /// The branch does not exist yet.
    Absent,
    /// The branch is at this commit.
    At(CommitId),
}

/// Why a forge operation failed.
#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
    /// The branch does not exist on the forge.
    #[error("branch {0} not found")]
    BranchNotFound(BranchName),
    /// The branch is not as the push expected.
    #[error("branch {0} moved")]
    Moved(BranchName),
    /// The commit is not in the mirror.
    #[error("commit {0} not found")]
    CommitNotFound(CommitId),
    /// git failed for another reason; the message is git's, without credentials.
    #[error("git failed: {0}")]
    Git(String),
}
