use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use igloo_core::repo::{BranchName, CommitId};

use crate::commit::{Commit, Signature};
use crate::config::ConfigKey;
use crate::diff::{DiffEntry, DiffStatus};
use crate::error::GitError;
use crate::file_diff::{DiffOutputs, FileDiff};
use crate::git::Git;
use crate::invocation::{Invocation, Output};
use crate::path::RepoPath;
use crate::refs::{Head, RefName, Refspec};
use crate::remote::{RemoteName, RemoteUrl};
use crate::transfer::{Endpoint, Lease, PushRejection};

/// A git repository, bare or with a work tree, driven through a [`Git`].
///
/// Invariant: its directory was a git repository when it was opened or created. Operations
/// on commits the repository lacks fail with [`GitError::Failed`]; check with
/// [`Repository::contains`] first where that must be told apart.
#[derive(Clone, Debug)]
pub struct Repository {
    git: Git,
    dir: PathBuf,
}

impl Repository {
    pub(crate) const fn new(git: Git, dir: PathBuf) -> Self {
        Self { git, dir }
    }

    /// Its directory: the work tree, or the git directory of a bare repository.
    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// What `HEAD` points at.
    pub async fn head(&self) -> Result<Head, GitError> {
        let symbolic = self
            .invoke("symbolic-ref")
            .arg("--quiet")
            .arg("HEAD")
            .output()
            .await?;
        match symbolic.status {
            Some(0) => {
                let name = symbolic
                    .text()?
                    .parse::<RefName>()
                    .ok()
                    .and_then(|name| name.branch())
                    .ok_or_else(|| symbolic.unexpected("HEAD is not a branch"))?;
                let commit = self.resolve(&RefName::head()).await?;
                Ok(Head::Branch { name, commit })
            }
            Some(1) => match self.resolve(&RefName::head()).await? {
                Some(commit) => Ok(Head::Detached(commit)),
                None => Err(symbolic.unexpected("a detached HEAD without a commit")),
            },
            _ => Err(symbolic.into_error()),
        }
    }

    /// The commit `name` points at, if it exists and points at one.
    pub async fn resolve(&self, name: &RefName) -> Result<Option<CommitId>, GitError> {
        self.verify(&format!("{name}^{{commit}}")).await
    }

    /// Whether the repository has `commit`.
    pub async fn contains(&self, commit: &CommitId) -> Result<bool, GitError> {
        Ok(self
            .verify(&format!("{commit}^{{commit}}"))
            .await?
            .is_some())
    }

    /// The commit `id`.
    pub async fn commit(&self, id: &CommitId) -> Result<Commit, GitError> {
        let output = self
            .log()
            .arg("-1")
            .arg(id.as_str())
            .arg("--")
            .run()
            .await?;
        Self::parse_commits(&output)?
            .pop()
            .ok_or_else(|| output.unexpected("no commit printed"))
    }

    /// The author of commit `id`.
    pub async fn author(&self, id: &CommitId) -> Result<Signature, GitError> {
        let output = self
            .invoke("log")
            .arg("-1")
            .arg("--no-color")
            .arg("--encoding=UTF-8")
            .arg("--format=%an%x00%ae%x00%at")
            .arg(id.as_str())
            .arg("--")
            .run()
            .await?;
        let text = output.text()?;
        let mut fields = text.split('\0');
        match (
            fields.next(),
            fields.next(),
            fields.next().map(str::parse::<i64>),
        ) {
            (Some(name), Some(email), Some(Ok(seconds))) => {
                Ok(Signature::new(name, email, seconds))
            }
            _ => Err(output.unexpected(format!("not an author: {text}"))),
        }
    }

    /// Creates a commit holding `tree_of`'s tree with the single parent `parent` and `message`,
    /// by `author` and `committer`; returns it. Equal inputs give the same commit.
    pub async fn commit_tree(
        &self,
        tree_of: &CommitId,
        parent: &CommitId,
        message: &str,
        author: &Signature,
        committer: &Signature,
    ) -> Result<CommitId, GitError> {
        let output = self
            .invoke("commit-tree")
            .arg(format!("{tree_of}^{{tree}}"))
            .arg("-p")
            .arg(parent.as_str())
            .arg("-m")
            .arg(message)
            .env("GIT_AUTHOR_NAME", author.name())
            .env("GIT_AUTHOR_EMAIL", author.email())
            .env("GIT_AUTHOR_DATE", author.date())
            .env("GIT_COMMITTER_NAME", committer.name())
            .env("GIT_COMMITTER_EMAIL", committer.email())
            .env("GIT_COMMITTER_DATE", committer.date())
            .run()
            .await?;
        Self::parse_commit(&output)
    }

    /// The commits reachable from `to` and not from `from`, oldest first.
    pub async fn commits(&self, from: &CommitId, to: &CommitId) -> Result<Vec<Commit>, GitError> {
        let output = self
            .log()
            .arg("--reverse")
            .arg(format!("{from}..{to}"))
            .arg("--")
            .run()
            .await?;
        Self::parse_commits(&output)
    }

    /// The best common ancestor of `a` and `b`; `None` when their histories are unrelated.
    pub async fn merge_base(
        &self,
        a: &CommitId,
        b: &CommitId,
    ) -> Result<Option<CommitId>, GitError> {
        let output = self
            .invoke("merge-base")
            .arg(a.as_str())
            .arg(b.as_str())
            .output()
            .await?;
        match output.status {
            Some(0) => Self::parse_commit(&output).map(Some),
            Some(1) if output.stderr.is_empty() => Ok(None),
            _ => Err(output.into_error()),
        }
    }

    /// The paths that differ between `from` and `to`, sorted by path.
    pub async fn diff(&self, from: &CommitId, to: &CommitId) -> Result<Vec<DiffEntry>, GitError> {
        let output = self
            .invoke("diff")
            .arg("--no-color")
            .arg("--no-ext-diff")
            .arg("--no-textconv")
            .arg("--no-renames")
            .arg("--name-status")
            .arg("-z")
            .arg(from.as_str())
            .arg(to.as_str())
            .arg("--")
            .run()
            .await?;
        let mut fields = output.stdout.split(|byte| *byte == 0);
        let mut entries = Vec::new();
        while let Some(status) = fields.next().filter(|status| !status.is_empty()) {
            let status = std::str::from_utf8(status)
                .ok()
                .and_then(DiffStatus::from_letter)
                .ok_or_else(|| output.unexpected("unknown diff status"))?;
            let path = fields
                .next()
                .map(|path| RepoPath::try_from(path.to_vec()))
                .ok_or_else(|| output.unexpected("a status without a path"))?
                .map_err(|_| output.unexpected("a path that is not UTF-8"))?;
            entries.push(DiffEntry::new(status, path));
        }
        entries.sort_by(|a, b| a.path().cmp(b.path()));
        Ok(entries)
    }

    /// The files that differ between `from` and `to`, sorted by path, with renames detected and
    /// the unified patch of each.
    pub async fn file_diffs(
        &self,
        from: &CommitId,
        to: &CommitId,
    ) -> Result<Vec<FileDiff>, GitError> {
        let run = |format: &[&str]| {
            let diff = self
                .invoke("diff")
                .arg("--no-color")
                .arg("--no-ext-diff")
                .arg("--no-textconv")
                .arg("--find-renames")
                .arg("--src-prefix=a/")
                .arg("--dst-prefix=b/");
            format
                .iter()
                .fold(diff, Invocation::arg)
                .arg(from.as_str())
                .arg(to.as_str())
                .arg("--")
                .run()
        };
        let name_status = run(&["--name-status", "-z"]).await?;
        let numstat = run(&["--numstat", "-z"]).await?;
        let patch = run(&["--patch"]).await?;
        let outputs = DiffOutputs {
            name_status: &name_status.stdout,
            numstat: &numstat.stdout,
            patch: &patch.stdout,
        };
        outputs.files().map_err(|reason| patch.unexpected(reason))
    }

    /// The content of the file at `path` in `commit`; `None` when there is no file there.
    pub async fn read_blob(
        &self,
        commit: &CommitId,
        path: &RepoPath,
    ) -> Result<Option<Vec<u8>>, GitError> {
        let listing = self
            .invoke("ls-tree")
            .arg("-z")
            .arg("--full-tree")
            .arg(commit.as_str())
            .arg("--")
            .arg(path.as_str())
            .run()
            .await?;
        let blob = listing
            .stdout
            .split(|byte| *byte == 0)
            .filter_map(|entry| std::str::from_utf8(entry).ok())
            .filter_map(|entry| entry.split_once('\t'))
            .find(|(_, name)| *name == path.as_str())
            .and_then(|(meta, _)| match meta.split(' ').collect::<Vec<_>>()[..] {
                [_, "blob", object] => Some(object.to_owned()),
                _ => None,
            });
        let Some(object) = blob else {
            return Ok(None);
        };
        let content = self
            .invoke("cat-file")
            .arg("blob")
            .arg(&object)
            .run()
            .await?;
        Ok(Some(content.stdout))
    }

    /// The URL of the remote `name`.
    pub async fn remote_url(&self, name: &RemoteName) -> Result<RemoteUrl, GitError> {
        let output = self
            .invoke("remote")
            .arg("get-url")
            .arg(name.as_str())
            .output()
            .await?;
        match output.status {
            Some(0) => output
                .text()?
                .parse()
                .map_err(|error| output.unexpected(format!("remote {name}: {error}"))),
            Some(2) => Err(GitError::NoSuchRemote(name.clone())),
            _ => Err(output.into_error()),
        }
    }

    /// Sets `key` to `value` in the repository's own configuration.
    pub async fn set_config(&self, key: &ConfigKey, value: &str) -> Result<(), GitError> {
        self.invoke("config")
            .arg(key.as_str())
            .arg(value)
            .run()
            .await
            .map(drop)
    }

    /// Fetches `refspecs` from `from`, without tags or submodules; only the last `depth`
    /// commits of each when given.
    ///
    /// Fails with [`GitError::RemoteRefNotFound`] when a source ref does not exist.
    pub async fn fetch(
        &self,
        from: &Endpoint,
        refspecs: &[Refspec],
        depth: Option<NonZeroU32>,
    ) -> Result<(), GitError> {
        let mut fetch = self
            .invoke("fetch")
            .arg("--quiet")
            .arg("--no-tags")
            .arg("--no-recurse-submodules");
        if let Some(depth) = depth {
            fetch = fetch.arg(format!("--depth={depth}"));
        }
        let mut fetch = Self::endpoint(fetch, from);
        for refspec in refspecs {
            fetch = fetch.arg(refspec.to_string());
        }
        fetch.run().await.map(drop)
    }

    /// Pushes `commit`, present in the repository, to `branch` on `to`, provided the branch is
    /// as `lease` expects. A branch already at `commit` is left as it is.
    ///
    /// Fails with [`GitError::Rejected`] when the remote refuses the update.
    pub async fn push(
        &self,
        to: &Endpoint,
        commit: &CommitId,
        branch: &BranchName,
        lease: &Lease,
    ) -> Result<(), GitError> {
        let target = RefName::from(branch);
        let lease = match lease {
            Lease::Any => "--force".to_owned(),
            Lease::Absent => format!("--force-with-lease={target}:"),
            Lease::At(current) => format!("--force-with-lease={target}:{current}"),
        };
        let push = self.invoke("push").arg("--porcelain").arg(lease);
        let output = Self::endpoint(push, to)
            .arg(format!("{commit}:{target}"))
            .output()
            .await?;
        if output.status == Some(0) {
            return Ok(());
        }
        let rejection = std::str::from_utf8(&output.stdout)
            .ok()
            .and_then(PushRejection::from_porcelain);
        Err(rejection.map_or_else(|| output.into_error(), GitError::Rejected))
    }

    /// Deletes `branch` at `to`, provided it is at `at`. A branch elsewhere, or already
    /// gone, is refused as [`PushRejection::Stale`].
    pub async fn delete_branch(
        &self,
        to: &Endpoint,
        branch: &BranchName,
        at: &CommitId,
    ) -> Result<(), GitError> {
        let target = RefName::from(branch);
        let push = self
            .invoke("push")
            .arg("--porcelain")
            .arg(format!("--force-with-lease={target}:{at}"));
        let output = Self::endpoint(push, to)
            .arg(format!(":{target}"))
            .output()
            .await?;
        if output.status == Some(0) {
            return Ok(());
        }
        let rejection = std::str::from_utf8(&output.stdout)
            .ok()
            .and_then(PushRejection::from_porcelain);
        Err(rejection.map_or_else(|| output.into_error(), GitError::Rejected))
    }

    /// Checks that the bundle at `bundle` is valid and its prerequisites are present.
    pub async fn verify_bundle(&self, bundle: &Path) -> Result<(), GitError> {
        self.invoke("bundle")
            .arg("verify")
            .arg("--quiet")
            .arg(Self::path_argument(bundle))
            .run()
            .await
            .map(drop)
    }

    /// Points `branch` at `commit`, creating or resetting it, and checks it out.
    pub async fn switch(&self, branch: &BranchName, commit: &CommitId) -> Result<(), GitError> {
        self.invoke("switch")
            .arg("--quiet")
            .arg("--no-guess")
            .arg("--force-create")
            .arg(branch.as_str())
            .arg(commit.as_str())
            .run()
            .await
            .map(drop)
    }

    /// Checks out `commit` detached from any branch.
    pub async fn checkout_detached(&self, commit: &CommitId) -> Result<(), GitError> {
        self.invoke("switch")
            .arg("--quiet")
            .arg("--detach")
            .arg(commit.as_str())
            .run()
            .await
            .map(drop)
    }

    fn invoke(&self, command: &'static str) -> Invocation<'_> {
        Invocation::new(&self.git, &self.dir, command)
    }

    /// `git log` printing each commit as `<id> NUL <message>`, commits separated by NUL.
    fn log(&self) -> Invocation<'_> {
        self.invoke("log")
            .arg("-z")
            .arg("--no-color")
            .arg("--no-show-signature")
            .arg("--encoding=UTF-8")
            .arg("--format=%H%x00%B")
    }

    /// The commit `revision` resolves to; `None` when it resolves to none.
    async fn verify(&self, revision: &str) -> Result<Option<CommitId>, GitError> {
        let output = self
            .invoke("rev-parse")
            .arg("--verify")
            .arg("--quiet")
            .arg(revision)
            .output()
            .await?;
        match output.status {
            Some(0) => Self::parse_commit(&output).map(Some),
            Some(1) => Ok(None),
            _ => Err(output.into_error()),
        }
    }

    fn endpoint<'a>(invocation: Invocation<'a>, endpoint: &Endpoint) -> Invocation<'a> {
        match endpoint {
            Endpoint::Remote(name) => invocation.arg(name.as_str()),
            Endpoint::Url { url, credentials } => {
                let invocation = match credentials {
                    Some(credentials) => {
                        invocation.config("http.extraHeader", credentials.header())
                    }
                    None => invocation,
                };
                invocation.arg(url.as_str())
            }
            Endpoint::Bundle(path) => invocation.arg(Self::path_argument(path)),
        }
    }

    /// `path`, prefixed with `./` when relative so that it never reads as an option.
    fn path_argument(path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_owned()
        } else {
            Path::new(".").join(path)
        }
    }

    fn parse_commit(output: &Output) -> Result<CommitId, GitError> {
        let text = output.text()?;
        text.parse()
            .map_err(|_| output.unexpected(format!("not a commit id: {text}")))
    }

    fn parse_commits(output: &Output) -> Result<Vec<Commit>, GitError> {
        let text = std::str::from_utf8(&output.stdout)
            .map_err(|_| output.unexpected("a commit message is not UTF-8"))?;
        let mut fields = text.split('\0');
        let mut commits = Vec::new();
        while let Some(id) = fields.next().filter(|id| !id.is_empty()) {
            let id = id
                .trim_start_matches('\n')
                .parse()
                .map_err(|_| output.unexpected(format!("not a commit id: {id}")))?;
            let message = fields
                .next()
                .ok_or_else(|| output.unexpected("a commit without a message"))?;
            commits.push(Commit::new(id, message));
        }
        Ok(commits)
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use igloo_core::repo::{BranchName, CommitId};

    use super::*;
    use crate::file_diff::FileStatus;
    use crate::git::Layout;
    use crate::testing::Fixture;

    fn branch(name: &str) -> BranchName {
        name.parse().expect("branch")
    }

    fn origin_url(fixture: &Fixture) -> Endpoint {
        Endpoint::url(fixture.origin().display().to_string().parse().expect("url"))
    }

    async fn mirror(dir: &std::path::Path) -> Repository {
        Git::isolated()
            .init(dir.join("mirror.git"), Layout::Bare, None)
            .await
            .expect("init")
    }

    #[tokio::test]
    async fn head_tells_branches_unborn_and_detached_apart() {
        let dir = tempfile::tempdir().expect("dir");
        let fixture = Fixture::new(dir.path());
        let work = Git::user().open(fixture.work()).await.expect("open");
        assert_eq!(
            work.head().await.expect("head"),
            Head::Branch {
                name: branch("main"),
                commit: None
            }
        );
        let first = fixture.commit("first\n\nbody", &[("a", Some("a"))]);
        assert_eq!(work.head().await.expect("head").commit(), Some(&first));
        assert_eq!(
            work.commit(&first).await.expect("commit").subject(),
            "first"
        );
        assert_eq!(
            work.commit(&first).await.expect("commit").message(),
            "first\n\nbody"
        );

        work.checkout_detached(&first).await.expect("detach");
        assert_eq!(
            work.head().await.expect("head"),
            Head::Detached(first.clone())
        );
        work.switch(&branch("feature"), &first)
            .await
            .expect("switch");
        assert_eq!(
            work.head().await.expect("head").branch(),
            Some(&branch("feature"))
        );
    }

    #[tokio::test]
    async fn fetches_land_refs_and_report_missing_ones() {
        let dir = tempfile::tempdir().expect("dir");
        let fixture = Fixture::new(dir.path());
        let first = fixture.commit("first", &[("a", Some("a"))]);
        let mirror = mirror(dir.path()).await;
        let main = branch("main");
        mirror
            .fetch(
                &origin_url(&fixture),
                &[Refspec::new(&main).to(&main).forced()],
                None,
            )
            .await
            .expect("fetch");
        assert_eq!(
            mirror
                .resolve(&RefName::from(&main))
                .await
                .expect("resolve"),
            Some(first.clone())
        );
        assert!(mirror.contains(&first).await.expect("contains"));
        let unknown: CommitId = "0".repeat(40).parse().expect("commit");
        assert!(!mirror.contains(&unknown).await.expect("contains"));
        let missing = branch("missing");
        let fetched = mirror
            .fetch(
                &origin_url(&fixture),
                &[Refspec::new(&missing).to(&missing)],
                None,
            )
            .await;
        assert!(
            matches!(fetched, Err(GitError::RemoteRefNotFound)),
            "{fetched:?}"
        );

        let shallow = Git::isolated()
            .init(dir.path().join("shallow"), Layout::WorkTree, None)
            .await
            .expect("init");
        let second = fixture.commit("second", &[("b", Some("b"))]);
        shallow
            .fetch(
                &origin_url(&fixture),
                &[Refspec::new(second.clone())],
                NonZeroU32::new(1),
            )
            .await
            .expect("fetch a commit by id");
        shallow.checkout_detached(&second).await.expect("checkout");
        assert_eq!(std::fs::read(shallow.path().join("b")).expect("b"), b"b");
        assert!(
            !shallow.contains(&first).await.expect("contains"),
            "the fetch is shallow"
        );
    }

    #[tokio::test]
    async fn pushes_honour_their_lease() {
        let dir = tempfile::tempdir().expect("dir");
        let fixture = Fixture::new(dir.path());
        let first = fixture.commit("first", &[("a", Some("a"))]);
        let mirror = mirror(dir.path()).await;
        let origin = origin_url(&fixture);
        let main = branch("main");
        mirror
            .fetch(&origin, &[Refspec::new(&main).to(&main)], None)
            .await
            .expect("fetch");
        let change = branch("igloo/changes/1");

        mirror
            .push(&origin, &first, &change, &Lease::Absent)
            .await
            .expect("create");
        assert_eq!(fixture.head("igloo/changes/1"), first);
        mirror
            .push(&origin, &first, &change, &Lease::Absent)
            .await
            .expect("a branch already at the commit is left as it is");

        let second = fixture.commit("second", &[("b", Some("b"))]);
        mirror
            .fetch(&origin, &[Refspec::new(&main).to(&main).forced()], None)
            .await
            .expect("fetch");
        let again = mirror.push(&origin, &second, &change, &Lease::Absent).await;
        assert!(
            matches!(again, Err(GitError::Rejected(PushRejection::Stale))),
            "{again:?}"
        );
        let stale = mirror
            .push(&origin, &first, &main, &Lease::At(first.clone()))
            .await;
        assert!(
            matches!(stale, Err(GitError::Rejected(PushRejection::Stale))),
            "{stale:?}"
        );
        mirror
            .push(&origin, &first, &main, &Lease::At(second.clone()))
            .await
            .expect("a lease on the current head");
        assert_eq!(fixture.head("main"), first);
        mirror
            .push(&origin, &second, &main, &Lease::Any)
            .await
            .expect("overwrite");
        assert_eq!(fixture.head("main"), second);
    }

    #[tokio::test]
    async fn history_diffs_and_files_are_read_from_commits() {
        let dir = tempfile::tempdir().expect("dir");
        let fixture = Fixture::new(dir.path());
        let base = fixture.commit("base", &[("keep", Some("1")), ("dir/old", Some("old"))]);
        let next = fixture.commit(
            "next\n\nwith a body",
            &[
                ("keep", Some("2")),
                ("dir/old", None),
                ("new file", Some("new")),
            ],
        );
        let last = fixture.commit("last", &[]);
        let repo = Git::isolated().open(fixture.origin()).await.expect("open");

        let commits = repo.commits(&base, &last).await.expect("log");
        let summary: Vec<_> = commits
            .iter()
            .map(|c| (c.id().clone(), c.message().to_owned()))
            .collect();
        assert_eq!(
            summary,
            [
                (next.clone(), "next\n\nwith a body".to_owned()),
                (last.clone(), "last".to_owned())
            ]
        );

        let diff = repo.diff(&base, &next).await.expect("diff");
        let diff: Vec<_> = diff
            .iter()
            .map(|e| (e.status(), e.path().as_str()))
            .collect();
        assert_eq!(
            diff,
            [
                (DiffStatus::Deleted, "dir/old"),
                (DiffStatus::Modified, "keep"),
                (DiffStatus::Added, "new file"),
            ]
        );

        let read = |commit: CommitId, path: &str| {
            let repo = repo.clone();
            let path = path.parse().expect("path");
            async move { repo.read_blob(&commit, &path).await.expect("read") }
        };
        assert_eq!(read(base.clone(), "dir/old").await, Some(b"old".to_vec()));
        assert_eq!(read(next.clone(), "dir/old").await, None);
        assert_eq!(
            read(base.clone(), "dir").await,
            None,
            "a directory is not a file"
        );
        assert_eq!(read(next.clone(), "new file").await, Some(b"new".to_vec()));

        assert_eq!(
            repo.merge_base(&next, &last).await.expect("merge base"),
            Some(next.clone())
        );
        fixture.git(&["checkout", "--quiet", "--orphan", "unrelated"]);
        let unrelated = fixture.commit("unrelated", &[]);
        assert_eq!(
            repo.merge_base(&base, &unrelated)
                .await
                .expect("merge base"),
            None
        );
    }

    #[tokio::test]
    async fn remotes_resolve_to_urls() {
        let dir = tempfile::tempdir().expect("dir");
        let fixture = Fixture::new(dir.path());
        fixture.git(&[
            "remote",
            "add",
            "origin",
            "git@github.com:roushou/igloo.git",
        ]);
        let work = Git::user().open(fixture.work()).await.expect("open");
        let url = work
            .remote_url(&RemoteName::origin())
            .await
            .expect("origin");
        assert_eq!(
            url,
            "git@github.com:roushou/igloo.git"
                .parse::<RemoteUrl>()
                .expect("url")
        );
        let missing = work.remote_url(&"upstream".parse().expect("name")).await;
        assert!(
            matches!(missing, Err(GitError::NoSuchRemote(_))),
            "{missing:?}"
        );
    }

    #[tokio::test]
    async fn bundles_are_verified_and_fetched() {
        let dir = tempfile::tempdir().expect("dir");
        let fixture = Fixture::new(dir.path());
        let base = fixture.commit("base", &[]);
        let mirror = mirror(dir.path()).await;
        let main = branch("main");
        mirror
            .fetch(
                &origin_url(&fixture),
                &[Refspec::new(&main).to(&main)],
                None,
            )
            .await
            .expect("fetch");
        fixture.git(&["commit", "--quiet", "--allow-empty", "-m", "bundled"]);
        let head: CommitId = fixture.git(&["rev-parse", "HEAD"]).parse().expect("commit");
        let bundle = dir.path().join("task.bundle");
        let path = bundle.display().to_string();
        fixture.git(&[
            "bundle",
            "create",
            "--quiet",
            &path,
            &format!("{base}..HEAD"),
        ]);

        mirror.verify_bundle(&bundle).await.expect("verify");
        let imported: RefName = "refs/igloo/imported".parse().expect("ref");
        mirror
            .fetch(
                &Endpoint::Bundle(bundle.clone()),
                &[Refspec::new(RefName::head()).to(imported.clone()).forced()],
                None,
            )
            .await
            .expect("fetch the bundle");
        assert_eq!(
            mirror.resolve(&imported).await.expect("resolve"),
            Some(head)
        );

        std::fs::write(&bundle, "not a bundle").expect("write");
        let garbage = mirror.verify_bundle(&bundle).await;
        assert!(
            matches!(garbage, Err(GitError::Failed { .. })),
            "{garbage:?}"
        );
    }

    #[tokio::test]
    async fn a_squash_commit_holds_the_tree_on_one_parent_and_is_deterministic() {
        let dir = tempfile::tempdir().expect("dir");
        let fixture = Fixture::new(dir.path());
        let base = fixture.commit("base", &[("keep", Some("1"))]);
        fixture.commit("one", &[("a", Some("a"))]);
        let head = fixture.commit("two", &[("b", Some("b"))]);
        let repo = Git::isolated().open(fixture.origin()).await.expect("open");

        let author = repo.author(&head).await.expect("author").at(1_700_000_000);
        let committer = Signature::new("Igloo", "igloo@igloo.invalid", 1_700_000_000);
        let squash = || repo.commit_tree(&head, &base, "Squashed\n\nbody", &author, &committer);
        let commit = squash().await.expect("commit-tree");
        assert_eq!(
            squash().await.expect("again"),
            commit,
            "equal inputs, same commit"
        );

        assert_eq!(repo.diff(&head, &commit).await.expect("diff"), []);
        let parents = fixture.git(&[
            "-C",
            &fixture.origin().display().to_string(),
            "rev-list",
            "--parents",
            "-n",
            "1",
            commit.as_str(),
        ]);
        assert_eq!(parents, format!("{commit} {base}"));
        let squashed = repo.commit(&commit).await.expect("commit");
        assert_eq!(squashed.message(), "Squashed\n\nbody");
        let recorded = repo.author(&commit).await.expect("author");
        assert_eq!(
            (recorded.name(), recorded.seconds()),
            (author.name(), 1_700_000_000)
        );
    }

    #[tokio::test]
    async fn file_diffs_carry_status_counts_and_patch() {
        let dir = tempfile::tempdir().expect("dir");
        let fixture = Fixture::new(dir.path());
        let body = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n";
        let base = fixture.commit(
            "base",
            &[
                ("keep", Some("1\n2\n")),
                ("gone", Some("bye\n")),
                ("old name", Some(body)),
                ("blob", Some("a\0b")),
                ("link", Some("plain\n")),
            ],
        );
        std::fs::remove_file(fixture.work().join("link")).expect("remove");
        #[cfg(unix)]
        std::os::unix::fs::symlink("keep", fixture.work().join("link")).expect("symlink");
        let next = fixture.commit(
            "next",
            &[
                ("keep", Some("1\n2\n3\n")),
                ("gone", None),
                ("old name", None),
                (
                    "dir/new name",
                    Some("one\ntwo\nthree\nfour\nfive\nsix\nseven\nEIGHT\n"),
                ),
                ("added", Some("hello\n")),
                ("blob", Some("a\0c")),
            ],
        );
        let repo = Git::isolated().open(fixture.origin()).await.expect("open");

        let files = repo.file_diffs(&base, &next).await.expect("file diffs");
        let summary: Vec<_> = files
            .iter()
            .map(|file| {
                (
                    file.path().as_str(),
                    file.previous().map(RepoPath::as_str),
                    file.status(),
                    (file.additions(), file.deletions()),
                    file.is_binary(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("added", None, FileStatus::Added, (1, 0), false),
                ("blob", None, FileStatus::Modified, (0, 0), true),
                (
                    "dir/new name",
                    Some("old name"),
                    FileStatus::Renamed,
                    (1, 1),
                    false
                ),
                ("gone", None, FileStatus::Deleted, (0, 1), false),
                ("keep", None, FileStatus::Modified, (1, 0), false),
                ("link", None, FileStatus::Modified, (1, 1), false),
            ]
        );
        let patch = |path: &str| {
            files
                .iter()
                .find(|file| file.path().as_str() == path)
                .map(FileDiff::patch)
                .expect("file")
        };
        assert_eq!(
            patch("keep"),
            "diff --git a/keep b/keep\nindex 1191247..01e79c3 100644\n--- a/keep\n+++ b/keep\n\
             @@ -1,2 +1,3 @@\n 1\n 2\n+3\n"
        );
        assert_eq!(patch("blob"), "");
        assert!(patch("dir/new name").starts_with("diff --git a/old name b/dir/new name\n"));
        assert!(patch("dir/new name").contains("similarity index"));
        assert!(patch("gone").contains("deleted file mode"));
        assert_eq!(
            patch("link").matches("diff --git").count(),
            2,
            "a change of kind prints as a removal and an addition"
        );

        assert_eq!(repo.file_diffs(&next, &next).await.expect("same"), []);
    }
}
