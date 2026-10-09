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

/// Who made a commit, and when, as git records it for an author or a committer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    name: String,
    email: String,
    seconds: i64,
}

impl Signature {
    /// `name <email>` at `seconds` since the Unix epoch, in UTC.
    pub fn new(name: impl Into<String>, email: impl Into<String>, seconds: i64) -> Self {
        Self {
            name: name.into(),
            email: email.into(),
            seconds,
        }
    }

    /// The same person at another time.
    #[must_use]
    pub fn at(mut self, seconds: i64) -> Self {
        self.seconds = seconds;
        self
    }

    /// The name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The email address.
    pub fn email(&self) -> &str {
        &self.email
    }

    /// Seconds since the Unix epoch.
    pub const fn seconds(&self) -> i64 {
        self.seconds
    }

    /// The date as git reads it from `GIT_*_DATE`.
    pub(crate) fn date(&self) -> String {
        format!("@{} +0000", self.seconds)
    }
}
