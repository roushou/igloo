//! Changes: a branch proposed for a repository's target branch, revised as it moves.

use igloo_core::change::{self as domain, Change, Comment, Revision};
use igloo_core::repo::BranchName;
use std::str::FromStr;

use igloo_core::{Actor, Entity, Timestamp, ValidationErrors, Validator};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::list::UnknownValue;

/// Opens a change for a branch pushed to the forge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct OpenChangeRequest {
    /// The branch proposed.
    pub branch: String,
    /// What the change does, 1 to 200 characters.
    pub title: String,
}

/// Where a change is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChangePhase {
    /// Taking revisions.
    Open,
    /// Merged; see `merged_commit`.
    Merged,
    /// Closed without merging.
    Closed,
}

/// One revision of a change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct RevisionResource {
    /// Its position, from 1.
    pub number: u32,
    /// The source branch's head.
    pub head: String,
    /// Where it forked from the target branch.
    pub base: String,
    /// When it was recorded.
    #[schema(value_type = String, format = DateTime)]
    pub created_at: Timestamp,
}

/// A human's approval of a revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct ApprovalResource {
    /// The revision approved.
    pub revision: u32,
    /// Who approved it (`usr_...`).
    pub by: String,
    /// When.
    #[schema(value_type = String, format = DateTime)]
    pub at: Timestamp,
}

/// A reviewer's comment on a revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct CommentResource {
    /// The revision commented on.
    pub revision: u32,
    /// Who wrote it: a user (`usr_...`), an agent (`agt_...`) or `system`.
    pub author: String,
    /// What it says.
    pub body: String,
    /// The file it is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The line of `path` it is about, from 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// When.
    #[schema(value_type = String, format = DateTime)]
    pub at: Timestamp,
}

/// Comments on a change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct CommentRequest {
    /// What it says; 1 to 10 000 characters.
    pub body: String,
    /// The revision; the latest when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u32>,
    /// The file it is about, relative to the repository root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The line of `path` it is about, from 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// Approves a change's latest revision.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct ApproveRequest {
    /// The revision approved; it must be the latest. The latest when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u32>,
}

/// Where the checks of a change's latest revision are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChecksState {
    /// The run passed.
    Passed,
    /// The run failed or errored.
    Failed,
    /// The run has not ended.
    Running,
    /// No run exists for the revision.
    Missing,
}

/// Whether a human must approve a change before it merges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ApprovalState {
    /// The change touches no protected path.
    NotRequired,
    /// The change touches protected paths and no human approved the latest revision.
    Required,
    /// The change touches protected paths and a human approved the latest revision.
    Given,
}

/// The approval a change needs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct ApprovalNeed {
    /// Whether it is needed and given.
    pub state: ApprovalState,
    /// The protected paths the change touches; empty when `state` is `not_required`.
    #[serde(default)]
    pub protected_paths: Vec<String>,
}

/// What stands between an open change and its merge, by the rules merging applies. Merging
/// stays authoritative: the target may move after this was computed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct MergeReadiness {
    /// The checks of the latest revision.
    pub checks: ChecksState,
    /// The approval of the latest revision.
    pub approval: ApprovalNeed,
    /// Whether the latest revision's base is the target branch's head, so it merges as is.
    pub fast_forward: bool,
}

/// A change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct ChangeResource {
    /// Its id (`chg_...`).
    pub id: String,
    /// The repository.
    pub repo: String,
    /// The branch proposed.
    pub source_branch: String,
    /// The branch it merges into.
    pub target_branch: String,
    /// What it does.
    pub title: String,
    /// Where it is.
    pub phase: ChangePhase,
    /// The target branch's head after the merge, once merged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_commit: Option<String>,
    /// Every revision, oldest first.
    pub revisions: Vec<RevisionResource>,
    /// Human approvals, oldest first.
    pub approvals: Vec<ApprovalResource>,
    /// Review comments, oldest first.
    #[serde(default)]
    pub comments: Vec<CommentResource>,
    /// Merge readiness; present only while the change is open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<MergeReadiness>,
    /// When it was merged; present only once merged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub merged_at: Option<Timestamp>,
    /// When it was closed; present only once closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub closed_at: Option<Timestamp>,
}

impl CommentRequest {
    /// A comment saying `body` on the latest revision.
    #[must_use]
    pub fn new(body: impl Into<String>) -> Self {
        Self {
            body: body.into(),
            revision: None,
            path: None,
            line: None,
        }
    }

    /// Places it on `revision`.
    #[must_use]
    pub const fn on_revision(mut self, revision: u32) -> Self {
        self.revision = Some(revision);
        self
    }

    /// Places it on `path`, and on `line` of it when given.
    #[must_use]
    pub fn on_path(mut self, path: impl Into<String>, line: Option<u32>) -> Self {
        self.path = Some(path.into());
        self.line = line;
        self
    }
}

impl From<&Comment> for CommentResource {
    fn from(comment: &Comment) -> Self {
        let author = match comment.author {
            Actor::Human { user } => user.to_string(),
            Actor::Agent { agent, .. } => agent.to_string(),
            Actor::System { .. } => "system".to_owned(),
        };
        Self {
            revision: comment.revision,
            author,
            body: comment.body.clone(),
            path: comment.path.clone(),
            line: comment.line,
            at: comment.at,
        }
    }
}

impl OpenChangeRequest {
    /// A request proposing `branch` as `title`.
    #[must_use]
    pub fn new(branch: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            branch: branch.into(),
            title: title.into(),
        }
    }

    /// The branch proposed, validated.
    pub fn source(&self) -> Result<BranchName, ValidationErrors> {
        let (branch,) = Validator::new()
            .field("branch", self.branch.parse::<BranchName>())
            .finish()?;
        Ok(branch)
    }
}

impl From<&Revision> for RevisionResource {
    fn from(revision: &Revision) -> Self {
        Self {
            number: revision.number,
            head: revision.head.to_string(),
            base: revision.base.to_string(),
            created_at: revision.at,
        }
    }
}

impl From<&Change> for ChangeResource {
    fn from(change: &Change) -> Self {
        let (phase, merged_commit) = match change.phase() {
            domain::ChangePhase::Open => (ChangePhase::Open, None),
            domain::ChangePhase::Merged { commit, .. } => {
                (ChangePhase::Merged, Some(commit.to_string()))
            }
            domain::ChangePhase::Closed => (ChangePhase::Closed, None),
        };
        Self {
            id: change.id().to_string(),
            repo: change.repo().to_string(),
            source_branch: change.source().to_string(),
            target_branch: change.target().to_string(),
            title: change.title().to_owned(),
            phase,
            merged_commit,
            revisions: change.revisions().map(RevisionResource::from).collect(),
            approvals: change
                .approvals()
                .iter()
                .map(|approval| ApprovalResource {
                    revision: approval.revision,
                    by: approval.by.to_string(),
                    at: approval.at,
                })
                .collect(),
            comments: change
                .comments()
                .iter()
                .map(CommentResource::from)
                .collect(),
            readiness: None,
            merged_at: None,
            closed_at: None,
        }
    }
}

impl FromStr for ChangePhase {
    type Err = UnknownValue;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "open" => Ok(Self::Open),
            "merged" => Ok(Self::Merged),
            "closed" => Ok(Self::Closed),
            other => Err(UnknownValue(other.to_owned())),
        }
    }
}

impl ChangeResource {
    /// Sets when it was merged.
    #[must_use]
    pub const fn with_merged_at(mut self, at: Timestamp) -> Self {
        self.merged_at = Some(at);
        self
    }

    /// Sets when it was closed.
    #[must_use]
    pub const fn with_closed_at(mut self, at: Timestamp) -> Self {
        self.closed_at = Some(at);
        self
    }

    /// Sets the merge readiness.
    #[must_use]
    pub fn with_readiness(mut self, readiness: MergeReadiness) -> Self {
        self.readiness = Some(readiness);
        self
    }
}

impl MergeReadiness {
    /// Readiness with the given parts.
    #[must_use]
    pub const fn new(checks: ChecksState, approval: ApprovalNeed, fast_forward: bool) -> Self {
        Self {
            checks,
            approval,
            fast_forward,
        }
    }
}

impl ApprovalNeed {
    /// An approval of `state` over `protected_paths`.
    #[must_use]
    pub const fn new(state: ApprovalState, protected_paths: Vec<String>) -> Self {
        Self {
            state,
            protected_paths,
        }
    }
}

/// How a file differs in a change's diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FileChangeStatus {
    /// Absent before, present after.
    Added,
    /// Present in both with different content, mode or kind.
    Modified,
    /// Present before, absent after.
    Deleted,
    /// Moved from `previous_path`, possibly with changes.
    Renamed,
}

/// One file of a change's diff.
///
/// A file has a `patch` unless it is binary or `truncated`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct FileDiffResource {
    /// Its path after the change; for a deleted file, the path it had.
    pub path: String,
    /// The path a renamed file had before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_path: Option<String>,
    /// How it differs.
    pub status: FileChangeStatus,
    /// Lines added; 0 for a binary file.
    pub additions: u64,
    /// Lines removed; 0 for a binary file.
    pub deletions: u64,
    /// Whether the file is binary.
    pub binary: bool,
    /// Whether the patch was left out for exceeding [`FileDiffResource::MAX_PATCH_BYTES`].
    pub truncated: bool,
    /// The unified patch exactly as git prints it, from its `diff --git` header on. Absent for
    /// a binary file and when `truncated`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
}

impl FileDiffResource {
    /// The longest patch returned, in bytes.
    pub const MAX_PATCH_BYTES: usize = 256 * 1024;

    /// A file with no patch yet: neither binary nor truncated.
    #[must_use]
    pub fn new(
        path: impl Into<String>,
        previous_path: Option<String>,
        status: FileChangeStatus,
        additions: u64,
        deletions: u64,
    ) -> Self {
        Self {
            path: path.into(),
            previous_path,
            status,
            additions,
            deletions,
            binary: false,
            truncated: false,
            patch: None,
        }
    }

    /// Marks the file binary: it has no patch.
    #[must_use]
    pub const fn binary(mut self) -> Self {
        self.binary = true;
        self
    }

    /// Gives the file `patch`, or marks it truncated and leaves the patch out when it exceeds
    /// [`FileDiffResource::MAX_PATCH_BYTES`].
    #[must_use]
    pub fn with_patch(mut self, patch: String) -> Self {
        if patch.len() > Self::MAX_PATCH_BYTES {
            self.truncated = true;
        } else {
            self.patch = Some(patch);
        }
        self
    }
}

/// What one revision of a change changes, relative to where it forked from the target branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct DiffResource {
    /// The revision, from 1.
    pub revision: u32,
    /// The commit the diff starts from: the revision's base.
    pub base: String,
    /// The commit the diff ends at: the revision's head.
    pub head: String,
    /// The files that differ, sorted by path.
    pub files: Vec<FileDiffResource>,
}

impl DiffResource {
    /// The diff of `revision` with `files`.
    #[must_use]
    pub fn new(revision: &Revision, files: Vec<FileDiffResource>) -> Self {
        Self {
            revision: revision.number,
            base: revision.base.to_string(),
            head: revision.head.to_string(),
            files,
        }
    }
}

/// Which revision of a change a diff is of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DiffQuery {
    /// The revision, from 1; the latest when omitted.
    pub revision: Option<u32>,
}
