use std::fmt;
use std::str::FromStr;

use crate::error::GitValueError;

/// A path inside a repository, relative to its root, with `/` separators.
///
/// Invariant: non-empty UTF-8, at most [`RepoPath::MAX_BYTES`] bytes, no NUL, no leading or
/// trailing `/`, and no empty, `.` or `..` component.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepoPath(String);

impl RepoPath {
    /// The longest path accepted, in bytes.
    pub const MAX_BYTES: usize = 4096;

    /// The path.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RepoPath {
    type Error = GitValueError;

    fn try_from(path: String) -> Result<Self, Self::Error> {
        let valid = !path.is_empty()
            && path.len() <= Self::MAX_BYTES
            && !path.contains('\0')
            && path
                .split('/')
                .all(|component| !matches!(component, "" | "." | ".."));
        if valid {
            Ok(Self(path))
        } else {
            Err(GitValueError::Path)
        }
    }
}

impl TryFrom<Vec<u8>> for RepoPath {
    type Error = GitValueError;

    /// Paths that are not UTF-8 are refused rather than altered.
    fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
        String::from_utf8(bytes)
            .map_err(|_| GitValueError::Path)
            .and_then(Self::try_from)
    }
}

impl FromStr for RepoPath {
    type Err = GitValueError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_owned())
    }
}

impl AsRef<str> for RepoPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<RepoPath> for String {
    fn from(path: RepoPath) -> Self {
        path.0
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_relative_and_normalized() {
        for valid in ["a", "a/b.txt", ".igloo/pipeline.toml", "a b/c", "..a"] {
            assert!(valid.parse::<RepoPath>().is_ok(), "{valid}");
        }
        for invalid in ["", "/a", "a/", "a//b", "./a", "a/../b", "..", "a\0b"] {
            assert!(invalid.parse::<RepoPath>().is_err(), "{invalid:?}");
        }
        assert!(RepoPath::try_from(vec![b'a', 0xff]).is_err());
    }
}
