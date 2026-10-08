use std::fmt;
use std::path::PathBuf;

use igloo_core::repo::CommitId;

use crate::remote::{Credentials, RemoteName, RemoteUrl};

/// Where a fetch or a push goes.
#[derive(Clone, Debug)]
pub enum Endpoint {
    /// A remote configured in the repository.
    Remote(RemoteName),
    /// A URL, with credentials for HTTP remotes that need them.
    Url {
        /// The remote.
        url: RemoteUrl,
        /// Sent to HTTP remotes; ignored by other schemes.
        credentials: Option<Credentials>,
    },
    /// A bundle file, for fetches only.
    Bundle(PathBuf),
}

/// What a push expects of the branch it updates on the remote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lease {
    /// Anything: the branch is overwritten.
    Any,
    /// The branch does not exist yet.
    Absent,
    /// The branch is at this commit.
    At(CommitId),
}

/// Why a remote refused a push.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushRejection {
    /// The branch is not as the lease expected.
    Stale,
    /// The push is not a fast-forward and was not forced.
    NotFastForward,
    /// The remote's own policy refused it (a hook, branch protection); git's reason.
    Remote(String),
}

impl Endpoint {
    /// The URL `url`, without credentials.
    pub const fn url(url: RemoteUrl) -> Self {
        Self::Url {
            url,
            credentials: None,
        }
    }
}

impl From<RemoteName> for Endpoint {
    fn from(name: RemoteName) -> Self {
        Self::Remote(name)
    }
}

impl From<RemoteUrl> for Endpoint {
    fn from(url: RemoteUrl) -> Self {
        Self::url(url)
    }
}

impl PushRejection {
    /// The rejection reported by `git push --porcelain` in `stdout`, if any ref was rejected.
    pub(crate) fn from_porcelain(stdout: &str) -> Option<Self> {
        let line = stdout.lines().find(|line| line.starts_with('!'))?;
        let summary = line.splitn(3, '\t').nth(2).unwrap_or_default();
        let reason = summary
            .rsplit_once('(')
            .map_or("", |(_, reason)| reason.trim_end_matches(')'));
        Some(if summary.starts_with("[remote rejected]") {
            Self::Remote(reason.to_owned())
        } else if reason == "stale info" {
            Self::Stale
        } else {
            Self::NotFastForward
        })
    }
}

impl fmt::Display for PushRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => f.write_str("the branch is not where the lease expected"),
            Self::NotFastForward => f.write_str("not a fast-forward"),
            Self::Remote(reason) => write!(f, "refused by the remote: {reason}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_rejections_are_classified() {
        let line = |summary: &str| format!("To /srv/r.git\n!\tabc:refs/heads/x\t{summary}\nDone\n");
        assert_eq!(
            PushRejection::from_porcelain(&line("[rejected] (stale info)")),
            Some(PushRejection::Stale)
        );
        assert_eq!(
            PushRejection::from_porcelain(&line("[rejected] (fetch first)")),
            Some(PushRejection::NotFastForward)
        );
        assert_eq!(
            PushRejection::from_porcelain(&line("[remote rejected] (protected branch)")),
            Some(PushRejection::Remote("protected branch".to_owned()))
        );
        assert_eq!(
            PushRejection::from_porcelain(
                "To /srv/r.git\n=\tabc:refs/heads/x\t[up to date]\nDone\n"
            ),
            None
        );
    }
}
