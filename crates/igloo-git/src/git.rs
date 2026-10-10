use std::path::{Path, PathBuf};
use std::time::Duration;

use igloo_core::repo::BranchName;

use crate::error::GitError;
use crate::invocation::Invocation;
use crate::repository::Repository;

/// How git runs: the executable, the environment it sees and how long it may take.
///
/// Invariant: every invocation through it is bounded by its timeout.
#[derive(Clone, Debug)]
pub struct Git {
    program: PathBuf,
    environment: Environment,
    timeout: Duration,
}

/// The environment git runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Environment {
    /// A person's: their global and system configuration apply, and git may prompt on the
    /// terminal for credentials.
    User,
    /// A service's: no global or system configuration, no prompts, no askpass helpers, no
    /// hooks, and only the `file`, `http`, `https` and `ssh` protocols.
    Isolated,
}

/// Whether a repository has a work tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// A bare repository: the directory is the git directory.
    Bare,
    /// A work tree with its `.git` directory.
    WorkTree,
}

impl Git {
    /// The default bound on one invocation: 10 minutes.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

    /// git on the `PATH` in a person's environment.
    pub fn user() -> Self {
        Self::new(Environment::User)
    }

    /// git on the `PATH` in an isolated environment, for services.
    pub fn isolated() -> Self {
        Self::new(Environment::Isolated)
    }

    fn new(environment: Environment) -> Self {
        Self {
            program: PathBuf::from("git"),
            environment,
            timeout: Self::DEFAULT_TIMEOUT,
        }
    }

    /// Runs the executable at `program` instead.
    #[must_use]
    pub fn with_program(mut self, program: impl Into<PathBuf>) -> Self {
        self.program = program.into();
        self
    }

    /// Bounds each invocation by `timeout`.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The executable.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// The environment.
    pub const fn environment(&self) -> Environment {
        self.environment
    }

    /// The bound on each invocation.
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The repository at `dir`, or the one containing it.
    ///
    /// Fails with [`GitError::NotARepository`] when there is none.
    pub async fn open(&self, dir: impl Into<PathBuf>) -> Result<Repository, GitError> {
        let dir = dir.into();
        Invocation::new(self, &dir, "rev-parse")
            .arg("--git-dir")
            .run()
            .await?;
        Ok(Repository::new(self.clone(), dir))
    }

    /// Creates a repository in `dir`, creating the directory if needed, with `initial` as the
    /// unborn branch of its `HEAD` (git's default when `None`). Re-initializing an existing
    /// repository keeps its history.
    pub async fn init(
        &self,
        dir: impl Into<PathBuf>,
        layout: Layout,
        initial: Option<&BranchName>,
    ) -> Result<Repository, GitError> {
        let dir = dir.into();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(GitError::Io)?;
        let mut init = Invocation::new(self, &dir, "init").arg("--quiet");
        if layout == Layout::Bare {
            init = init.arg("--bare");
        }
        if let Some(branch) = initial {
            init = init.arg(format!("--initial-branch={branch}"));
        }
        init.run().await?;
        Ok(Repository::new(self.clone(), dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Fixture;

    #[tokio::test]
    async fn opening_a_directory_outside_a_repository_fails() {
        let dir = tempfile::tempdir().expect("dir");
        let opened = Git::isolated().open(dir.path()).await;
        assert!(
            matches!(opened, Err(GitError::NotARepository(_))),
            "{opened:?}"
        );
        let missing = Git::isolated().open(dir.path().join("missing")).await;
        assert!(
            matches!(missing, Err(GitError::NotARepository(_))),
            "{missing:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn invocations_time_out() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("dir");
        let fixture = Fixture::new(dir.path());
        // A git that never answers in time, so the outcome does not depend on how fast git is.
        let program = dir.path().join("slow-git");
        std::fs::write(&program, "#!/bin/sh\nsleep 30\n").expect("script");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let git = Git::isolated()
            .with_program(program)
            .with_timeout(std::time::Duration::from_millis(100));
        // A process another test forks while the script is being written can hold it open for
        // writing until it execs, which makes running it fail with "text file busy" on Linux.
        let opened = loop {
            match git.open(fixture.work()).await {
                Err(GitError::Spawn(error))
                    if error.kind() == std::io::ErrorKind::ExecutableFileBusy =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                opened => break opened,
            }
        };
        assert!(
            matches!(opened, Err(GitError::Timeout { .. })),
            "{opened:?}"
        );
    }
}
