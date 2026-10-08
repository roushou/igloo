use crate::path::RepoPath;

/// One path that differs between two commits. Renames and copies are reported as a deletion
/// and an addition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffEntry {
    status: DiffStatus,
    path: RepoPath,
}

/// How a path differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffStatus {
    /// Absent before, present after.
    Added,
    /// Present before, absent after.
    Deleted,
    /// Present in both with different content or mode.
    Modified,
    /// Present in both as a different kind of object (file, symlink, submodule).
    TypeChanged,
}

impl DiffEntry {
    /// `path` differing as `status`.
    pub const fn new(status: DiffStatus, path: RepoPath) -> Self {
        Self { status, path }
    }

    /// How it differs.
    pub const fn status(&self) -> DiffStatus {
        self.status
    }

    /// The path.
    pub const fn path(&self) -> &RepoPath {
        &self.path
    }
}

impl From<DiffEntry> for RepoPath {
    fn from(entry: DiffEntry) -> Self {
        entry.path
    }
}

impl DiffStatus {
    /// The status of git's `--name-status` letter, if it is one of the reported kinds.
    pub(crate) const fn from_letter(letter: &str) -> Option<Self> {
        match letter.as_bytes() {
            b"A" => Some(Self::Added),
            b"D" => Some(Self::Deleted),
            b"M" => Some(Self::Modified),
            b"T" => Some(Self::TypeChanged),
            _ => None,
        }
    }
}
