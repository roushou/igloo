use std::fmt;
use std::str::FromStr;

use igloo_core::repo::{BranchName, CommitId};

use crate::error::GitValueError;

/// A full ref name: `HEAD` or `refs/<name>`.
///
/// Invariant: `HEAD`, or `refs/` followed by a name that follows the [`BranchName`] rules.
/// It never starts with `-`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RefName(String);

/// What a refspec takes: a ref or a commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefSource {
    /// A ref, resolved where the refspec is applied.
    Ref(RefName),
    /// A commit, by id.
    Commit(CommitId),
}

/// A fetch refspec: `[+]<source>[:<destination>]`.
///
/// Without a destination the fetched objects are kept but no ref is updated. A forced refspec
/// updates its destination even when the update is not a fast-forward.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refspec {
    source: RefSource,
    destination: Option<RefName>,
    force: bool,
}

/// What `HEAD` points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Head {
    /// A branch; `commit` is `None` while the branch is unborn.
    Branch {
        /// The branch.
        name: BranchName,
        /// Its commit.
        commit: Option<CommitId>,
    },
    /// A commit, detached from any branch.
    Detached(CommitId),
}

impl RefName {
    /// `HEAD`.
    pub fn head() -> Self {
        Self("HEAD".to_owned())
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The branch this ref names, if it is under `refs/heads/`.
    pub fn branch(&self) -> Option<BranchName> {
        self.0.strip_prefix("refs/heads/")?.parse().ok()
    }
}

impl From<&BranchName> for RefName {
    /// `refs/heads/<branch>`.
    fn from(branch: &BranchName) -> Self {
        Self(format!("refs/heads/{branch}"))
    }
}

impl TryFrom<String> for RefName {
    type Error = GitValueError;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        let valid = name == "HEAD"
            || name
                .strip_prefix("refs/")
                .is_some_and(|rest| rest.parse::<BranchName>().is_ok());
        if valid {
            Ok(Self(name))
        } else {
            Err(GitValueError::RefName)
        }
    }
}

impl FromStr for RefName {
    type Err = GitValueError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_owned())
    }
}

impl fmt::Display for RefName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<RefName> for RefSource {
    fn from(name: RefName) -> Self {
        Self::Ref(name)
    }
}

impl From<&BranchName> for RefSource {
    fn from(branch: &BranchName) -> Self {
        Self::Ref(branch.into())
    }
}

impl From<CommitId> for RefSource {
    fn from(commit: CommitId) -> Self {
        Self::Commit(commit)
    }
}

impl fmt::Display for RefSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ref(name) => name.fmt(f),
            Self::Commit(commit) => commit.fmt(f),
        }
    }
}

impl Refspec {
    /// Takes `source` without updating any ref.
    pub fn new(source: impl Into<RefSource>) -> Self {
        Self {
            source: source.into(),
            destination: None,
            force: false,
        }
    }

    /// Stores what it takes in `destination`.
    #[must_use]
    pub fn to(mut self, destination: impl Into<RefName>) -> Self {
        self.destination = Some(destination.into());
        self
    }

    /// Updates the destination even when the update is not a fast-forward.
    #[must_use]
    pub const fn forced(mut self) -> Self {
        self.force = true;
        self
    }

    /// What it takes.
    pub const fn source(&self) -> &RefSource {
        &self.source
    }

    /// Where it stores it.
    pub const fn destination(&self) -> Option<&RefName> {
        self.destination.as_ref()
    }
}

impl fmt::Display for Refspec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.force {
            f.write_str("+")?;
        }
        write!(f, "{}", self.source)?;
        if let Some(destination) = &self.destination {
            write!(f, ":{destination}")?;
        }
        Ok(())
    }
}

impl Head {
    /// The branch, unless detached.
    pub const fn branch(&self) -> Option<&BranchName> {
        match self {
            Self::Branch { name, .. } => Some(name),
            Self::Detached(_) => None,
        }
    }

    /// The commit, unless on an unborn branch.
    pub const fn commit(&self) -> Option<&CommitId> {
        match self {
            Self::Branch { commit, .. } => commit.as_ref(),
            Self::Detached(commit) => Some(commit),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_names_are_head_or_under_refs() {
        for valid in [
            "HEAD",
            "refs/heads/main",
            "refs/igloo/imported",
            "refs/tags/v1.0",
        ] {
            assert!(valid.parse::<RefName>().is_ok(), "{valid}");
        }
        for invalid in [
            "main",
            "FETCH_HEAD",
            "refs/",
            "refs/heads/a..b",
            "-refs/x",
            "refs/a b",
        ] {
            assert!(invalid.parse::<RefName>().is_err(), "{invalid}");
        }
        let main: BranchName = "main".parse().unwrap();
        assert_eq!(RefName::from(&main).as_str(), "refs/heads/main");
        assert_eq!(RefName::from(&main).branch(), Some(main));
    }

    #[test]
    fn refspecs_render_as_git_reads_them() {
        let main: BranchName = "main".parse().unwrap();
        let commit: CommitId = "a".repeat(40).parse().unwrap();
        assert_eq!(
            Refspec::new(&main).to(&main).forced().to_string(),
            "+refs/heads/main:refs/heads/main"
        );
        assert_eq!(Refspec::new(commit.clone()).to_string(), commit.to_string());
        assert_eq!(
            Refspec::new(RefName::head()).to(&main).to_string(),
            "HEAD:refs/heads/main"
        );
    }
}
