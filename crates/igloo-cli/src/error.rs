//! Why a command failed.

use igloo_git::GitError;

/// Why a command failed: the server, the local repository, or how the command was used.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CliError {
    /// The API call failed.
    #[error(transparent)]
    Api(#[from] igloo::Error),
    /// git failed in the local repository.
    #[error("git: {0}")]
    Git(#[from] GitError),
    /// A local file or stream could not be read.
    #[error("local I/O failure: {0}")]
    Io(#[from] std::io::Error),
    /// The command cannot run as given; the message says what to do instead.
    #[error("{0}")]
    Usage(String),
}
