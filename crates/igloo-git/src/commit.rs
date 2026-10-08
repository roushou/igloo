use igloo_core::repo::CommitId;

/// A commit and its message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    id: CommitId,
    message: String,
}

impl Commit {
    /// The commit `id` with `message`, trailing whitespace removed.
    pub fn new(id: CommitId, message: &str) -> Self {
        Self {
            id,
            message: message.trim_end().to_owned(),
        }
    }

    /// Its id.
    pub const fn id(&self) -> &CommitId {
        &self.id
    }

    /// Its full message, without trailing whitespace.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The first line of its message.
    pub fn subject(&self) -> &str {
        self.message.lines().next().unwrap_or_default()
    }
}

impl From<Commit> for CommitId {
    fn from(commit: Commit) -> Self {
        commit.id
    }
}
