//! A git client driving the `git` binary.
//!
//! [`Git`] says how git runs: which executable, in which environment, for how long.
//! [`Repository`] is one repository opened through it. Values passed to git are typed
//! ([`RefName`], [`RemoteUrl`], [`RepoPath`], `CommitId`, `BranchName`), so no argument can be
//! read as an option, and outputs are parsed from machine-readable formats.
//!
//! Invariants of every invocation:
//! - git runs with the C locale, so its messages are stable;
//! - git never reads repository location variables (`GIT_DIR`, `GIT_WORK_TREE`, ...) from the
//!   caller's environment: the repository is the one named by its directory;
//! - pathspecs are literal;
//! - credentials travel in git's environment, never its arguments;
//! - git is killed when its future is dropped or its timeout elapses.

mod commit;
mod config;
mod diff;
mod error;
mod git;
mod invocation;
mod path;
mod refs;
mod remote;
mod repository;
mod transfer;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use commit::Commit;
pub use config::ConfigKey;
pub use diff::{DiffEntry, DiffStatus};
pub use error::{GitError, GitValueError};
pub use git::{Environment, Git, Layout};
pub use path::RepoPath;
pub use refs::{Head, RefName, RefSource, Refspec};
pub use remote::{Credentials, RemoteName, RemoteUrl, Scheme};
pub use repository::Repository;
pub use transfer::{Endpoint, Lease, PushRejection};
