use std::path::PathBuf;
use std::time::Duration;

use igloo_core::ErrorCode;

use crate::remote::RemoteName;
use crate::transfer::PushRejection;

/// Why a git operation failed.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// The git executable could not be started.
    #[error("could not run git")]
    Spawn(#[source] std::io::Error),
    /// git ran longer than its timeout and was killed.
    #[error("git {command} timed out after {timeout:?}")]
    Timeout {
        /// The subcommand.
        command: &'static str,
        /// The timeout that elapsed.
        timeout: Duration,
    },
    /// The directory is not a git repository, or does not exist.
    #[error("{} is not a git repository", .0.display())]
    NotARepository(PathBuf),
    /// The repository has no remote of that name.
    #[error("no remote named {0}")]
    NoSuchRemote(RemoteName),
    /// A fetched ref does not exist on the remote.
    #[error("the remote has no such ref")]
    RemoteRefNotFound,
    /// The remote refused the credentials, or none were available.
    #[error("authentication with the remote failed")]
    Authentication,
    /// The remote refused a push.
    #[error("push rejected: {0}")]
    Rejected(PushRejection),
    /// git failed for another reason; `stderr` is git's message.
    #[error("git {command} failed ({}): {stderr}", status.map_or_else(|| "killed".to_owned(), |code| format!("exit {code}")))]
    Failed {
        /// The subcommand.
        command: &'static str,
        /// Its exit code; `None` when killed by a signal.
        status: Option<i32>,
        /// What it wrote to stderr, trimmed.
        stderr: String,
    },
    /// git succeeded but printed something this crate cannot parse.
    #[error("unexpected output from git {command}: {detail}")]
    Unexpected {
        /// The subcommand.
        command: &'static str,
        /// What was wrong with it.
        detail: String,
    },
    /// A file system operation around git failed.
    #[error("file system failure")]
    Io(#[source] std::io::Error),
}

impl ErrorCode for GitError {
    fn code(&self) -> &'static str {
        match self {
            Self::Spawn(_) => "git.spawn_failed",
            Self::Timeout { .. } => "git.timeout",
            Self::NotARepository(_) => "git.not_a_repository",
            Self::NoSuchRemote(_) => "git.no_such_remote",
            Self::RemoteRefNotFound => "git.remote_ref_not_found",
            Self::Authentication => "git.authentication_failed",
            Self::Rejected(_) => "git.push_rejected",
            Self::Failed { .. } => "git.failed",
            Self::Unexpected { .. } => "git.unexpected_output",
            Self::Io(_) => "git.io",
        }
    }
}

/// Why a value cannot be passed to git.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GitValueError {
    /// Not `HEAD` or a valid `refs/...` name.
    #[error("not a valid ref name: HEAD or refs/<name>")]
    RefName,
    /// Not a valid remote name.
    #[error("not a valid remote name: [A-Za-z0-9._-], not starting with - or .")]
    RemoteName,
    /// Not a URL git can fetch from with an allowed protocol.
    #[error("not a supported remote URL: https, http, ssh, scp-like or an absolute path")]
    RemoteUrl,
    /// Not a relative path inside a repository.
    #[error("not a relative repository path without ., .. or empty components")]
    Path,
    /// Not a `section[.subsection].name` configuration key.
    #[error("not a configuration key: section[.subsection].name")]
    ConfigKey,
}

impl ErrorCode for GitValueError {
    fn code(&self) -> &'static str {
        match self {
            Self::RefName => "git.invalid_ref_name",
            Self::RemoteName => "git.invalid_remote_name",
            Self::RemoteUrl => "git.invalid_remote_url",
            Self::Path => "git.invalid_path",
            Self::ConfigKey => "git.invalid_config_key",
        }
    }
}
