//! Repositories: where code lives on a forge, and the trust boundary for everything done to it.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use crate::snapshot::SnapshotId;
use crate::{Digest, Entity, Event, Id, Prefixed, Timestamp};

/// Identifies a repository (`repo_...`).
pub type RepoId = Id<Repo>;

/// A repository Igloo works on.
///
/// Invariant: unique per location, enforced by the platform when registering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    id: RepoId,
    location: RepoLocation,
    default_branch: BranchName,
    token: Option<SecretName>,
    warm: BTreeMap<Digest, WarmSnapshot>,
    images: BTreeMap<String, SnapshotId>,
    used: BTreeMap<Digest, Timestamp>,
    registered_at: Timestamp,
    events: Vec<RepoEvent>,
}

/// A snapshot whose dependencies are built, reused by every commit with the same key: a digest
/// of what it was built from (base snapshot, build command, lockfiles).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WarmSnapshot {
    /// The sealed snapshot.
    pub snapshot: SnapshotId,
    /// The commit it was built at.
    pub commit: CommitId,
}

/// Where a repository's git history is hosted.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "forge", rename_all = "snake_case")]
pub enum RepoLocation {
    /// `github.com/<owner>/<name>`.
    Github {
        /// The owning user or organization.
        owner: String,
        /// The repository name.
        name: String,
    },
    /// A git repository on the server's file system, for development and tests.
    Local {
        /// Its absolute path.
        path: String,
    },
}

/// A git branch name: no whitespace or control characters, no `..`, `~`, `^`, `:`, `?`, `*`,
/// `[` or `\`, not starting with `-` or `/`, not ending with `/` or `.lock`, at most 255 bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BranchName(String);

/// A git commit id: 40 (SHA-1) or 64 (SHA-256) lowercase hex digits.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CommitId(String);

/// The name of a repository secret, used as the environment variable it is exposed as:
/// `[A-Z_][A-Z0-9_]*`, at most 64 bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SecretName(String);

/// Facts about a repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RepoEvent {
    /// The repository was registered.
    Registered {
        /// Its id.
        id: RepoId,
        /// Where it is hosted.
        location: RepoLocation,
        /// The branch changes merge into.
        default_branch: BranchName,
        /// The secret holding the forge token, if the forge needs one.
        token: Option<SecretName>,
        /// When.
        at: Timestamp,
    },
    /// A secret was set or replaced. Its value is never recorded.
    SecretSet {
        /// The secret.
        name: SecretName,
    },
    /// A secret was deleted.
    SecretDeleted {
        /// The secret.
        name: SecretName,
    },
    /// A container image was imported as the snapshot its checkouts go over.
    ImageImported {
        /// The image reference, such as `docker.io/library/rust:1.99-slim`, and its platform.
        image: String,
        /// The imported snapshot.
        snapshot: SnapshotId,
    },
    /// A warm snapshot was built for `key`, replacing any earlier one.
    WarmSnapshotRecorded {
        /// What it was built from.
        key: Digest,
        /// The snapshot and its commit.
        warm: WarmSnapshot,
    },
    /// The snapshot recorded under `key` was used: a checkout or a sandbox was made over it.
    WarmSnapshotUsed {
        /// The key.
        key: Digest,
        /// When.
        at: Timestamp,
    },
    /// The snapshot recorded under `key` was forgotten; the key is rebuilt when next needed.
    WarmSnapshotForgotten {
        /// The key.
        key: Digest,
    },
}

/// Why a repository value is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RepoValueError {
    /// Not `github.com/<owner>/<name>` or an absolute path.
    #[error("must be github.com/<owner>/<name> or an absolute path")]
    Location,
    /// Breaks the branch name rules.
    #[error("not a valid branch name")]
    Branch,
    /// Not a commit id.
    #[error("must be 40 or 64 lowercase hex digits")]
    Commit,
    /// Breaks the secret name rules.
    #[error("must match [A-Z_][A-Z0-9_]* and be at most 64 bytes")]
    Secret,
}

impl Repo {
    /// The shortest time, in seconds, between two recorded uses of one snapshot.
    pub const USE_INTERVAL_SECONDS: i64 = 3600;

    /// A newly registered repository.
    #[must_use]
    pub fn new(
        id: RepoId,
        location: RepoLocation,
        default_branch: BranchName,
        token: Option<SecretName>,
        now: Timestamp,
    ) -> Self {
        let mut repo = Self::initial(
            id,
            location.clone(),
            default_branch.clone(),
            token.clone(),
            now,
        );
        repo.events.push(RepoEvent::Registered {
            id,
            location,
            default_branch,
            token,
            at: now,
        });
        repo
    }

    fn initial(
        id: RepoId,
        location: RepoLocation,
        default_branch: BranchName,
        token: Option<SecretName>,
        at: Timestamp,
    ) -> Self {
        Self {
            id,
            location,
            default_branch,
            token,
            warm: BTreeMap::new(),
            images: BTreeMap::new(),
            used: BTreeMap::new(),
            registered_at: at,
            events: Vec::new(),
        }
    }

    /// Records that secret `name` was set.
    pub fn secret_set(&mut self, name: SecretName) {
        self.record(RepoEvent::SecretSet { name });
    }

    /// Records that secret `name` was deleted.
    pub fn secret_deleted(&mut self, name: SecretName) {
        self.record(RepoEvent::SecretDeleted { name });
    }

    /// Records `snapshot` as the import of `image`.
    pub fn image_imported(&mut self, image: String, snapshot: SnapshotId) {
        self.record(RepoEvent::ImageImported { image, snapshot });
    }

    /// The snapshot `image` was imported as, if it was.
    #[must_use]
    pub fn image(&self, image: &str) -> Option<SnapshotId> {
        self.images.get(image).copied()
    }

    /// Records `warm` as the warm snapshot for `key`.
    pub fn record_warm(&mut self, key: Digest, warm: WarmSnapshot) {
        self.record(RepoEvent::WarmSnapshotRecorded { key, warm });
    }

    /// The warm snapshot built for `key`, if any.
    #[must_use]
    pub fn warm(&self, key: &Digest) -> Option<&WarmSnapshot> {
        self.warm.get(key)
    }

    /// Every recorded warm snapshot (agent snapshots included) by key, in key order.
    pub fn warm_snapshots(&self) -> impl Iterator<Item = (&Digest, &WarmSnapshot)> {
        self.warm.iter()
    }

    /// Records that the snapshot under `key` was used at `now`. Nothing is recorded when `key`
    /// has no snapshot or a use was recorded less than [`Self::USE_INTERVAL_SECONDS`] before.
    pub fn warm_used(&mut self, key: Digest, now: Timestamp) {
        if !self.warm.contains_key(&key) {
            return;
        }
        let window = jiff::SignedDuration::from_secs(Self::USE_INTERVAL_SECONDS);
        let recent = self
            .used
            .get(&key)
            .is_some_and(|last| now.duration_since(*last) < window);
        if !recent {
            self.record(RepoEvent::WarmSnapshotUsed { key, at: now });
        }
    }

    /// Forgets the snapshot recorded under `key`, so its layers are no longer a root. Nothing is
    /// recorded unless a use was recorded and it was at or before `since`.
    pub fn forget_warm_unused_since(&mut self, key: Digest, since: Timestamp) {
        if self.used.get(&key).is_some_and(|last| *last <= since) {
            self.record(RepoEvent::WarmSnapshotForgotten { key });
        }
    }

    /// When the snapshot under `key` was last used; `None` when it has no snapshot or no use was
    /// recorded.
    #[must_use]
    pub fn warm_last_used(&self, key: &Digest) -> Option<Timestamp> {
        self.used.get(key).copied()
    }

    /// Every imported image's snapshot, in image order.
    pub fn image_snapshots(&self) -> impl Iterator<Item = SnapshotId> + '_ {
        self.images.values().copied()
    }

    fn record(&mut self, event: RepoEvent) {
        self.apply(&event);
        self.events.push(event);
    }

    /// Where it is hosted.
    #[must_use]
    pub const fn location(&self) -> &RepoLocation {
        &self.location
    }

    /// The branch changes merge into.
    #[must_use]
    pub const fn default_branch(&self) -> &BranchName {
        &self.default_branch
    }

    /// The secret holding the forge token, if any.
    #[must_use]
    pub const fn token(&self) -> Option<&SecretName> {
        self.token.as_ref()
    }

    /// When it was registered.
    #[must_use]
    pub const fn registered_at(&self) -> Timestamp {
        self.registered_at
    }
}

impl Prefixed for Repo {
    const PREFIX: &'static str = "repo";
}

impl Event for RepoEvent {
    const SCHEMA_VERSION: u16 = 1;

    fn kind(&self) -> &'static str {
        match self {
            Self::Registered { .. } => "igloo.repo.registered",
            Self::SecretSet { .. } => "igloo.repo.secret_set",
            Self::SecretDeleted { .. } => "igloo.repo.secret_deleted",
            Self::WarmSnapshotRecorded { .. } => "igloo.repo.warm_snapshot_recorded",
            Self::ImageImported { .. } => "igloo.repo.image_imported",
            Self::WarmSnapshotUsed { .. } => "igloo.repo.warm_snapshot_used",
            Self::WarmSnapshotForgotten { .. } => "igloo.repo.warm_snapshot_forgotten",
        }
    }
}

impl Entity for Repo {
    const NAME: &'static str = "repo";

    type Event = RepoEvent;

    fn id(&self) -> RepoId {
        self.id
    }

    fn from_created(event: &RepoEvent) -> Option<Self> {
        let RepoEvent::Registered {
            id,
            location,
            default_branch,
            token,
            at,
        } = event
        else {
            return None;
        };
        Some(Self::initial(
            *id,
            location.clone(),
            default_branch.clone(),
            token.clone(),
            *at,
        ))
    }

    fn apply(&mut self, event: &RepoEvent) {
        match event {
            RepoEvent::Registered { .. }
            | RepoEvent::SecretSet { .. }
            | RepoEvent::SecretDeleted { .. } => {}
            RepoEvent::WarmSnapshotRecorded { key, warm } => {
                self.warm.insert(*key, warm.clone());
            }
            RepoEvent::WarmSnapshotUsed { key, at } => {
                self.used.insert(*key, *at);
            }
            RepoEvent::WarmSnapshotForgotten { key } => {
                self.warm.remove(key);
                self.used.remove(key);
            }
            RepoEvent::ImageImported { image, snapshot } => {
                self.images.insert(image.clone(), *snapshot);
            }
        }
    }

    fn take_events(&mut self) -> Vec<RepoEvent> {
        std::mem::take(&mut self.events)
    }
}

impl FromStr for RepoLocation {
    type Err = RepoValueError;

    /// Parses `github.com/<owner>/<name>` (with an optional `https://` and `.git`) or an absolute
    /// path.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with('/') && !s.contains('\0') {
            return Ok(Self::Local { path: s.to_owned() });
        }
        let rest = s.strip_prefix("https://").unwrap_or(s);
        let rest = rest
            .strip_prefix("github.com/")
            .ok_or(RepoValueError::Location)?;
        let rest = rest.strip_suffix(".git").unwrap_or(rest);
        let (owner, name) = rest.split_once('/').ok_or(RepoValueError::Location)?;
        let valid = |part: &str| {
            !part.is_empty()
                && part.len() <= 100
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
                && !part.starts_with('.')
        };
        if !valid(owner) || !valid(name) {
            return Err(RepoValueError::Location);
        }
        Ok(Self::Github {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }
}

impl fmt::Display for RepoLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Github { owner, name } => write!(f, "github.com/{owner}/{name}"),
            Self::Local { path } => f.write_str(path),
        }
    }
}

impl BranchName {
    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for BranchName {
    type Error = RepoValueError;

    #[allow(
        clippy::case_sensitive_file_extension_comparisons,
        reason = "git reserves exactly the lowercase `.lock` suffix"
    )]
    fn try_from(name: String) -> Result<Self, Self::Error> {
        let forbidden = |c: char| {
            c.is_whitespace()
                || c.is_control()
                || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        };
        let valid = !name.is_empty()
            && name.len() <= 255
            && !name.contains(forbidden)
            && !name.contains("..")
            && !name.contains("//")
            && !name.contains("@{")
            && !name.starts_with(['-', '/', '.'])
            && !name.ends_with(['/', '.'])
            && !name.ends_with(".lock");
        if valid {
            Ok(Self(name))
        } else {
            Err(RepoValueError::Branch)
        }
    }
}

impl FromStr for BranchName {
    type Err = RepoValueError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_owned())
    }
}

impl From<BranchName> for String {
    fn from(name: BranchName) -> Self {
        name.0
    }
}

impl fmt::Display for BranchName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl CommitId {
    /// The hex digits.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CommitId {
    type Error = RepoValueError;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        let valid = matches!(id.len(), 40 | 64)
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if valid {
            Ok(Self(id))
        } else {
            Err(RepoValueError::Commit)
        }
    }
}

impl FromStr for CommitId {
    type Err = RepoValueError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.trim().to_owned())
    }
}

impl From<CommitId> for String {
    fn from(id: CommitId) -> Self {
        id.0
    }
}

impl fmt::Display for CommitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl SecretName {
    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SecretName {
    type Error = RepoValueError;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        let valid = name.len() <= 64
            && name
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_uppercase() || b == b'_')
            && name
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
        if valid {
            Ok(Self(name))
        } else {
            Err(RepoValueError::Secret)
        }
    }
}

impl FromStr for SecretName {
    type Err = RepoValueError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_owned())
    }
}

impl From<SecretName> for String {
    fn from(name: SecretName) -> Self {
        name.0
    }
}

impl fmt::Display for SecretName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Scenario;

    type S = Scenario<Repo>;

    #[test]
    fn locations_parse_github_and_local_forms() {
        let github = RepoLocation::Github {
            owner: "roushou".to_owned(),
            name: "igloo".to_owned(),
        };
        for form in [
            "github.com/roushou/igloo",
            "https://github.com/roushou/igloo.git",
        ] {
            assert_eq!(form.parse::<RepoLocation>(), Ok(github.clone()));
        }
        assert_eq!(github.to_string(), "github.com/roushou/igloo");
        assert!(matches!(
            "/srv/git/igloo.git".parse::<RepoLocation>(),
            Ok(RepoLocation::Local { .. })
        ));
        for invalid in [
            "gitlab.com/a/b",
            "github.com/a",
            "github.com/a/b/c",
            "relative",
        ] {
            assert!(invalid.parse::<RepoLocation>().is_err(), "{invalid}");
        }
    }

    #[test]
    fn branch_commit_and_secret_names_are_validated() {
        for valid in ["main", "igloo/changes/1", "release-1.2"] {
            assert!(valid.parse::<BranchName>().is_ok(), "{valid}");
        }
        for invalid in ["", "-x", "a..b", "a b", "x.lock", "a/", "refs~1"] {
            assert!(invalid.parse::<BranchName>().is_err(), "{invalid}");
        }
        assert!("a".repeat(40).parse::<CommitId>().is_ok());
        assert!("A".repeat(40).parse::<CommitId>().is_err());
        assert!("GITHUB_TOKEN".parse::<SecretName>().is_ok());
        assert!("github-token".parse::<SecretName>().is_err());
    }

    #[test]
    fn registration_records_the_repository() {
        let location: RepoLocation = "github.com/roushou/igloo".parse().expect("location");
        let main: BranchName = "main".parse().expect("branch");
        let token: SecretName = "GITHUB_TOKEN".parse().expect("secret");
        S::create(|now| {
            Repo::new(
                S::ID,
                location.clone(),
                main.clone(),
                Some(token.clone()),
                now,
            )
        })
        .then([RepoEvent::Registered {
            id: S::ID,
            location: location.clone(),
            default_branch: main.clone(),
            token: Some(token.clone()),
            at: S::NOW,
        }]);
    }

    #[test]
    fn a_warm_snapshot_is_found_by_its_key_and_replaced_by_a_newer_one() {
        let key = Digest::from_blake3([1; 32]);
        let warm = |n: u8| WarmSnapshot {
            snapshot: SnapshotId::from(Digest::from_blake3([n; 32])),
            commit: format!("{n:040x}").parse().expect("commit"),
        };
        let scenario = S::given([registered()])
            .when(|repo, _| repo.record_warm(key, warm(2)))
            .then([RepoEvent::WarmSnapshotRecorded { key, warm: warm(2) }])
            .when(|repo, _| repo.record_warm(key, warm(3)))
            .then([RepoEvent::WarmSnapshotRecorded { key, warm: warm(3) }]);
        assert_eq!(scenario.state().warm(&key), Some(&warm(3)));
        assert_eq!(scenario.state().warm(&Digest::from_blake3([9; 32])), None);
    }

    fn recorded(key: Digest) -> RepoEvent {
        RepoEvent::WarmSnapshotRecorded {
            key,
            warm: WarmSnapshot {
                snapshot: SnapshotId::from(Digest::from_blake3([2; 32])),
                commit: "a".repeat(40).parse().expect("commit"),
            },
        }
    }

    #[test]
    fn uses_are_recorded_at_most_once_an_hour_and_only_for_recorded_keys() {
        let key = Digest::from_blake3([1; 32]);
        let at = |minutes: i64| S::NOW.saturating_add(jiff::SignedDuration::from_mins(minutes));
        let scenario = S::given([registered(), recorded(key)])
            .when(|repo, _| repo.warm_used(Digest::from_blake3([9; 32]), at(0)))
            .then([])
            .when(|repo, _| repo.warm_used(key, at(0)))
            .then([RepoEvent::WarmSnapshotUsed { key, at: at(0) }])
            .when(|repo, _| repo.warm_used(key, at(59)))
            .then([])
            .when(|repo, _| repo.warm_used(key, at(60)))
            .then([RepoEvent::WarmSnapshotUsed { key, at: at(60) }]);
        assert_eq!(scenario.state().warm_last_used(&key), Some(at(60)));
    }

    #[test]
    fn a_snapshot_used_after_the_cutoff_is_kept_and_a_forgotten_one_is_dropped() {
        let key = Digest::from_blake3([1; 32]);
        let before = S::NOW.saturating_add(jiff::SignedDuration::from_secs(-1));
        let scenario = S::given([
            registered(),
            recorded(key),
            RepoEvent::WarmSnapshotUsed { key, at: S::NOW },
        ])
        .when(|repo, _| repo.forget_warm_unused_since(key, before))
        .then([])
        .when(|repo, _| repo.forget_warm_unused_since(key, S::NOW))
        .then([RepoEvent::WarmSnapshotForgotten { key }])
        .when(|repo, _| repo.forget_warm_unused_since(key, S::NOW))
        .then([]);
        assert_eq!(scenario.state().warm(&key), None);
        assert_eq!(scenario.state().warm_last_used(&key), None);
    }

    #[test]
    fn secret_changes_are_recorded_by_name() {
        let name: SecretName = "API_TOKEN".parse().expect("secret");
        S::given([registered()])
            .when(|repo, _| repo.secret_set(name.clone()))
            .then([RepoEvent::SecretSet { name: name.clone() }])
            .when(|repo, _| repo.secret_deleted(name.clone()))
            .then([RepoEvent::SecretDeleted { name: name.clone() }]);
    }

    fn registered() -> RepoEvent {
        RepoEvent::Registered {
            id: S::ID,
            location: "github.com/roushou/igloo".parse().expect("location"),
            default_branch: "main".parse().expect("branch"),
            token: Some("GITHUB_TOKEN".parse().expect("secret")),
            at: S::NOW,
        }
    }

    #[test]
    fn events_serialize_stably() {
        let events = vec![
            registered(),
            RepoEvent::SecretSet {
                name: "API_TOKEN".parse().expect("secret"),
            },
            RepoEvent::SecretDeleted {
                name: "API_TOKEN".parse().expect("secret"),
            },
            RepoEvent::WarmSnapshotRecorded {
                key: Digest::from_blake3([1; 32]),
                warm: WarmSnapshot {
                    snapshot: SnapshotId::from(Digest::from_blake3([2; 32])),
                    commit: "a".repeat(40).parse().expect("commit"),
                },
            },
            RepoEvent::WarmSnapshotUsed {
                key: Digest::from_blake3([1; 32]),
                at: S::NOW,
            },
            RepoEvent::WarmSnapshotForgotten {
                key: Digest::from_blake3([1; 32]),
            },
        ];
        insta::assert_json_snapshot!(events);
    }
}
