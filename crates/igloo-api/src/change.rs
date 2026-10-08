//! Changes: a branch proposed for a repository's target branch, revised as it moves.

use igloo_core::change::{self as domain, Change, Comment, Revision};
use igloo_core::repo::BranchName;
use igloo_core::{Actor, Entity, Timestamp, ValidationErrors, Validator};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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
        }
    }
}
