//! Changes: a branch's commits proposed for a repository's target branch, one revision per
//! pushed head.

use serde::{Deserialize, Serialize};

use crate::repo::{BranchName, CommitId, RepoId};
use crate::{Actor, Entity, ErrorCode, Event, Id, Prefixed, Timestamp, UserId};

/// Identifies a change (`chg_...`).
pub type ChangeId = Id<Change>;

/// A proposal to merge a branch into a repository's target branch.
///
/// Invariant: at least one revision; revisions only append, numbered from 1; a change ends once,
/// merged or closed, and takes no revision after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    id: ChangeId,
    repo: RepoId,
    source: BranchName,
    target: BranchName,
    title: String,
    earlier: Vec<Revision>,
    latest: Revision,
    approvals: Vec<Approval>,
    comments: Vec<Comment>,
    requested: usize,
    phase: ChangePhase,
    events: Vec<ChangeEvent>,
}

/// A reviewer's comment on a revision, optionally on a line of a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    /// The revision commented on.
    pub revision: u32,
    /// Who wrote it.
    pub author: Actor,
    /// What it says; 1 to 10 000 characters.
    pub body: String,
    /// The file it is about, relative to the repository root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The line of `path` it is about, from 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// When.
    pub at: Timestamp,
}

/// A reviewer asking for another revision, with the comments made since the previous request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeRequest {
    /// The revision reviewed: the latest when asked.
    pub revision: u32,
    /// Who asked.
    pub by: Actor,
    /// The comments it carries, oldest first; never empty.
    pub comments: Vec<Comment>,
    /// When.
    pub at: Timestamp,
}

/// A human's approval of one revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    /// The revision approved.
    pub revision: u32,
    /// Who approved it.
    pub by: UserId,
    /// When.
    pub at: Timestamp,
}

/// One state of a change: the source branch's head and where it forked from the target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    /// Its position, from 1.
    pub number: u32,
    /// The source branch's head.
    pub head: CommitId,
    /// The merge base of `head` and the target branch.
    pub base: CommitId,
    /// When it was recorded.
    pub at: Timestamp,
}

/// Where a change is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum ChangePhase {
    /// Taking revisions.
    Open,
    /// Merged into the target branch. Final.
    Merged {
        /// The revision merged.
        revision: u32,
        /// The target branch's head after the merge.
        commit: CommitId,
    },
    /// Closed without merging. Final.
    Closed,
}

/// Facts about a change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChangeEvent {
    /// The change was opened with its first revision.
    Opened {
        /// Its id.
        id: ChangeId,
        /// The repository.
        repo: RepoId,
        /// The branch proposed.
        source: BranchName,
        /// The branch it merges into.
        target: BranchName,
        /// Its title.
        title: String,
        /// Revision 1.
        revision: Revision,
    },
    /// A new head was recorded.
    Revised {
        /// The new revision.
        revision: Revision,
    },
    /// The change was closed without merging.
    Closed,
    /// A human approved a revision.
    Approved {
        /// The approval.
        approval: Approval,
    },
    /// A reviewer commented.
    Commented {
        /// The comment.
        comment: Comment,
    },
    /// A reviewer asked for another revision.
    ChangesRequested {
        /// The request.
        request: ChangeRequest,
    },
    /// The change was merged: its target branch now points at the revision's head.
    Merged {
        /// The revision merged.
        revision: u32,
        /// The target branch's new head.
        commit: CommitId,
    },
}

/// Why a change is rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChangeError {
    /// The source branch is the target branch.
    #[error("a change cannot propose its target branch")]
    SameBranch,
    /// The title is empty or longer than 200 characters.
    #[error("the title must be 1 to 200 characters")]
    Title,
    /// The change was merged or closed.
    #[error("the change has ended")]
    Ended,
    /// Only the latest revision is approved.
    #[error("only the latest revision is approved")]
    Approval,
    /// Only humans approve.
    #[error("only a human approves")]
    AgentApproval,
    /// The comment is empty or too long, or names no revision of the change.
    #[error("a comment needs 1 to 10000 characters and a revision of the change")]
    Comment,
    /// No comment was made since the previous request.
    #[error("comment before requesting changes")]
    NothingToRequest,
    /// The revision to merge is not the latest.
    #[error("only the latest revision merges")]
    StaleRevision,
}

impl ErrorCode for ChangeError {
    fn code(&self) -> &'static str {
        match self {
            Self::SameBranch => "change.same_branch",
            Self::Title => "change.invalid_title",
            Self::Ended => "change.ended",
            Self::Approval => "change.invalid_approval",
            Self::AgentApproval => "change.agent_approval",
            Self::Comment => "change.invalid_comment",
            Self::NothingToRequest => "change.nothing_to_request",
            Self::StaleRevision => "change.stale_revision",
        }
    }
}

impl Change {
    const MAX_TITLE: usize = 200;
    const MAX_COMMENT: usize = 10_000;

    /// A change proposing `source`'s `head`, forked from `target` at `base`.
    pub fn open(
        id: ChangeId,
        repo: RepoId,
        source: BranchName,
        target: BranchName,
        title: &str,
        (head, base): (CommitId, CommitId),
        now: Timestamp,
    ) -> Result<Self, ChangeError> {
        if source == target {
            return Err(ChangeError::SameBranch);
        }
        let title = title.trim().to_owned();
        if title.is_empty() || title.chars().count() > Self::MAX_TITLE {
            return Err(ChangeError::Title);
        }
        let revision = Revision {
            number: 1,
            head,
            base,
            at: now,
        };
        let mut change = Self::initial(
            id,
            repo,
            source.clone(),
            target.clone(),
            title.clone(),
            revision.clone(),
        );
        change.events.push(ChangeEvent::Opened {
            id,
            repo,
            source,
            target,
            title,
            revision,
        });
        Ok(change)
    }

    /// Records `head`, forked from the target at `base`, as the next revision. The current head
    /// again records nothing. Returns the latest revision's number.
    pub fn revise(
        &mut self,
        head: CommitId,
        base: CommitId,
        now: Timestamp,
    ) -> Result<u32, ChangeError> {
        if self.phase != ChangePhase::Open {
            return Err(ChangeError::Ended);
        }
        if self.latest.head == head {
            return Ok(self.latest.number);
        }
        let number = self.latest.number + 1;
        self.record(ChangeEvent::Revised {
            revision: Revision {
                number,
                head,
                base,
                at: now,
            },
        });
        Ok(number)
    }

    /// Records `actor`'s approval of `revision`, which must be the latest; only humans approve.
    /// Approving again records nothing.
    pub fn approve(
        &mut self,
        revision: u32,
        actor: Actor,
        now: Timestamp,
    ) -> Result<(), ChangeError> {
        if self.phase != ChangePhase::Open {
            return Err(ChangeError::Ended);
        }
        let Actor::Human { user } = actor else {
            return Err(ChangeError::AgentApproval);
        };
        if revision != self.latest.number {
            return Err(ChangeError::Approval);
        }
        if self.approved_by_human(revision) {
            return Ok(());
        }
        self.record(ChangeEvent::Approved {
            approval: Approval {
                revision,
                by: user,
                at: now,
            },
        });
        Ok(())
    }

    /// Whether a human approved `revision`.
    #[must_use]
    pub fn approved_by_human(&self, revision: u32) -> bool {
        self.approvals
            .iter()
            .any(|approval| approval.revision == revision)
    }

    /// Records that `revision`, the latest, was merged and the target branch is at `commit`.
    /// Merging the same revision again records nothing.
    pub fn merge(&mut self, revision: u32, commit: CommitId) -> Result<(), ChangeError> {
        match &self.phase {
            ChangePhase::Merged {
                revision: merged, ..
            } if *merged == revision => return Ok(()),
            ChangePhase::Open => {}
            _ => return Err(ChangeError::Ended),
        }
        if revision != self.latest.number {
            return Err(ChangeError::StaleRevision);
        }
        self.record(ChangeEvent::Merged { revision, commit });
        Ok(())
    }

    /// Records `comment`, on a revision of the open change.
    pub fn comment(&mut self, comment: Comment) -> Result<(), ChangeError> {
        if self.phase != ChangePhase::Open {
            return Err(ChangeError::Ended);
        }
        let length = comment.body.trim().chars().count();
        if length == 0
            || length > Self::MAX_COMMENT
            || comment.revision == 0
            || comment.revision > self.latest.number
            || comment.line == Some(0)
            || (comment.line.is_some() && comment.path.is_none())
        {
            return Err(ChangeError::Comment);
        }
        self.record(ChangeEvent::Commented { comment });
        Ok(())
    }

    /// Asks for another revision on behalf of `by`, carrying the comments made since the
    /// previous request; there must be some.
    pub fn request_changes(&mut self, by: Actor, now: Timestamp) -> Result<(), ChangeError> {
        if self.phase != ChangePhase::Open {
            return Err(ChangeError::Ended);
        }
        let comments = self.comments.get(self.requested..).unwrap_or_default();
        if comments.is_empty() {
            return Err(ChangeError::NothingToRequest);
        }
        self.record(ChangeEvent::ChangesRequested {
            request: ChangeRequest {
                revision: self.latest.number,
                by,
                comments: comments.to_vec(),
                at: now,
            },
        });
        Ok(())
    }

    /// Every comment, oldest first.
    #[must_use]
    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// Every approval, oldest first.
    #[must_use]
    pub fn approvals(&self) -> &[Approval] {
        &self.approvals
    }

    /// Closes the change without merging. Closing again does nothing; a merged change cannot
    /// close.
    pub fn close(&mut self) -> Result<(), ChangeError> {
        match self.phase {
            ChangePhase::Open => {
                self.record(ChangeEvent::Closed);
                Ok(())
            }
            ChangePhase::Closed => Ok(()),
            ChangePhase::Merged { .. } => Err(ChangeError::Ended),
        }
    }

    /// The repository.
    #[must_use]
    pub const fn repo(&self) -> RepoId {
        self.repo
    }

    /// The branch proposed.
    #[must_use]
    pub const fn source(&self) -> &BranchName {
        &self.source
    }

    /// The branch it merges into.
    #[must_use]
    pub const fn target(&self) -> &BranchName {
        &self.target
    }

    /// Its title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Every revision, oldest first.
    pub fn revisions(&self) -> impl Iterator<Item = &Revision> {
        self.earlier.iter().chain(std::iter::once(&self.latest))
    }

    /// The latest revision.
    #[must_use]
    pub const fn latest(&self) -> &Revision {
        &self.latest
    }

    fn initial(
        id: ChangeId,
        repo: RepoId,
        source: BranchName,
        target: BranchName,
        title: String,
        revision: Revision,
    ) -> Self {
        Self {
            id,
            repo,
            source,
            target,
            title,
            earlier: Vec::new(),
            latest: revision,
            approvals: Vec::new(),
            comments: Vec::new(),
            requested: 0,
            phase: ChangePhase::Open,
            events: Vec::new(),
        }
    }

    /// Where it is.
    #[must_use]
    pub const fn phase(&self) -> &ChangePhase {
        &self.phase
    }

    fn record(&mut self, event: ChangeEvent) {
        self.apply(&event);
        self.events.push(event);
    }
}

impl Prefixed for Change {
    const PREFIX: &'static str = "chg";
}

impl Event for ChangeEvent {
    const SCHEMA_VERSION: u16 = 1;

    fn kind(&self) -> &'static str {
        match self {
            Self::Opened { .. } => "igloo.change.opened",
            Self::Revised { .. } => "igloo.change.revised",
            Self::Closed => "igloo.change.closed",
            Self::Approved { .. } => "igloo.change.approved",
            Self::Commented { .. } => "igloo.change.commented",
            Self::ChangesRequested { .. } => "igloo.change.changes_requested",
            Self::Merged { .. } => "igloo.change.merged",
        }
    }
}

impl Entity for Change {
    const NAME: &'static str = "change";

    type Event = ChangeEvent;

    fn id(&self) -> ChangeId {
        self.id
    }

    fn from_created(event: &ChangeEvent) -> Option<Self> {
        let ChangeEvent::Opened {
            id,
            repo,
            source,
            target,
            title,
            revision,
        } = event
        else {
            return None;
        };
        Some(Self::initial(
            *id,
            *repo,
            source.clone(),
            target.clone(),
            title.clone(),
            revision.clone(),
        ))
    }

    fn apply(&mut self, event: &ChangeEvent) {
        match event {
            ChangeEvent::Opened { .. } => {}
            ChangeEvent::Revised { revision } => {
                let previous = std::mem::replace(&mut self.latest, revision.clone());
                self.earlier.push(previous);
            }
            ChangeEvent::Closed => self.phase = ChangePhase::Closed,
            ChangeEvent::Approved { approval } => self.approvals.push(approval.clone()),
            ChangeEvent::Commented { comment } => self.comments.push(comment.clone()),
            ChangeEvent::ChangesRequested { .. } => self.requested = self.comments.len(),
            ChangeEvent::Merged { revision, commit } => {
                self.phase = ChangePhase::Merged {
                    revision: *revision,
                    commit: commit.clone(),
                };
            }
        }
    }

    fn take_events(&mut self) -> Vec<ChangeEvent> {
        std::mem::take(&mut self.events)
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::testing::Scenario;

    type S = Scenario<Change>;

    fn commit(n: u8) -> CommitId {
        format!("{n:040x}").parse().expect("commit")
    }

    fn branch(name: &str) -> BranchName {
        name.parse().expect("branch")
    }

    fn revision(number: u32, head: u8) -> Revision {
        Revision {
            number,
            head: commit(head),
            base: commit(0),
            at: S::NOW,
        }
    }

    fn opened() -> ChangeEvent {
        ChangeEvent::Opened {
            id: S::ID,
            repo: Id::from_uuid(Uuid::from_u128(5)),
            source: branch("feature"),
            target: branch("main"),
            title: "Add a feature".to_owned(),
            revision: revision(1, 1),
        }
    }

    fn open(source: &str, title: &str) -> impl FnOnce(Timestamp) -> Result<Change, ChangeError> {
        let (source, title) = (branch(source), title.to_owned());
        move |now| {
            Change::open(
                S::ID,
                Id::from_uuid(Uuid::from_u128(5)),
                source,
                branch("main"),
                &title,
                (commit(1), commit(0)),
                now,
            )
        }
    }

    #[test]
    fn opening_records_the_first_revision() {
        S::try_create(open("feature", " Add a feature ")).then([opened()]);
    }

    #[test]
    fn a_change_cannot_propose_its_target_or_lack_a_title() {
        S::try_create(open("main", "x")).then_error("change.same_branch");
        S::try_create(open("feature", "  ")).then_error("change.invalid_title");
    }

    #[test]
    fn revisions_append_once_per_new_head() {
        S::given([opened()])
            .try_when(|change, now| change.revise(commit(2), commit(0), now))
            .then([ChangeEvent::Revised {
                revision: revision(2, 2),
            }])
            .try_when(|change, now| change.revise(commit(2), commit(0), now))
            .then_no_events();
    }

    #[test]
    fn a_closed_change_takes_no_revision() {
        S::given([opened()])
            .try_when(|change, _| change.close())
            .then([ChangeEvent::Closed])
            .try_when(|change, _| change.close())
            .then_no_events()
            .try_when(|change, now| change.revise(commit(3), commit(0), now))
            .then_error("change.ended");
    }

    #[test]
    fn humans_approve_the_latest_revision_once() {
        let human = Actor::Human {
            user: Id::from_uuid(Uuid::from_u128(7)),
        };
        let agent = Actor::Agent {
            agent: Id::from_uuid(Uuid::from_u128(8)),
            principal: Id::from_uuid(Uuid::from_u128(7)),
        };
        let approved = S::given([opened()])
            .try_when(|change, now| change.approve(1, agent, now))
            .then_error("change.agent_approval")
            .try_when(|change, now| change.approve(2, human, now))
            .then_error("change.invalid_approval")
            .try_when(|change, now| change.approve(1, human, now))
            .then([ChangeEvent::Approved {
                approval: Approval {
                    revision: 1,
                    by: Id::from_uuid(Uuid::from_u128(7)),
                    at: S::NOW,
                },
            }])
            .try_when(|change, now| change.approve(1, human, now))
            .then_no_events();
        assert!(approved.state().approved_by_human(1));
        let revised = approved
            .try_when(|change, now| change.revise(commit(2), commit(0), now))
            .then([ChangeEvent::Revised {
                revision: revision(2, 2),
            }]);
        assert!(
            !revised.state().approved_by_human(2),
            "an approval holds for its revision only"
        );
    }

    fn comment(revision: u32, body: &str) -> Comment {
        Comment {
            revision,
            author: Actor::Human {
                user: Id::from_uuid(Uuid::from_u128(7)),
            },
            body: body.to_owned(),
            path: Some("src/lib.rs".to_owned()),
            line: Some(3),
            at: S::NOW,
        }
    }

    #[test]
    fn a_request_for_changes_carries_the_comments_since_the_previous_one() {
        let reviewer = Actor::Human {
            user: Id::from_uuid(Uuid::from_u128(7)),
        };
        let requested = S::given([opened()])
            .try_when(|change, now| change.request_changes(reviewer, now))
            .then_error("change.nothing_to_request")
            .try_when(|change, _| change.comment(comment(1, "Handle the empty case")))
            .then([ChangeEvent::Commented {
                comment: comment(1, "Handle the empty case"),
            }])
            .try_when(|change, now| change.request_changes(reviewer, now))
            .then([ChangeEvent::ChangesRequested {
                request: ChangeRequest {
                    revision: 1,
                    by: reviewer,
                    comments: vec![comment(1, "Handle the empty case")],
                    at: S::NOW,
                },
            }])
            .try_when(|change, now| change.request_changes(reviewer, now))
            .then_error("change.nothing_to_request");
        requested
            .try_when(|change, _| change.comment(comment(1, "And add a test")))
            .then([ChangeEvent::Commented {
                comment: comment(1, "And add a test"),
            }])
            .try_when(|change, now| change.request_changes(reviewer, now))
            .then([ChangeEvent::ChangesRequested {
                request: ChangeRequest {
                    revision: 1,
                    by: reviewer,
                    comments: vec![comment(1, "And add a test")],
                    at: S::NOW,
                },
            }]);
    }

    #[test]
    fn a_comment_names_a_revision_and_has_a_body() {
        for invalid in [
            comment(2, "on a revision that does not exist"),
            comment(1, " "),
            Comment {
                path: None,
                ..comment(1, "a line of no file")
            },
            Comment {
                line: Some(0),
                ..comment(1, "line zero")
            },
        ] {
            S::given([opened()])
                .try_when(|change, _| change.comment(invalid))
                .then_error("change.invalid_comment");
        }
        S::given([opened(), ChangeEvent::Closed])
            .try_when(|change, _| change.comment(comment(1, "late")))
            .then_error("change.ended");
    }

    #[test]
    fn the_latest_revision_merges_once() {
        S::given([opened()])
            .try_when(|change, _| change.merge(1, commit(1)))
            .then([ChangeEvent::Merged {
                revision: 1,
                commit: commit(1),
            }])
            .try_when(|change, _| change.merge(1, commit(1)))
            .then_no_events()
            .try_when(|change, now| change.revise(commit(3), commit(0), now))
            .then_error("change.ended")
            .try_when(|change, _| change.close())
            .then_error("change.ended");
        S::given([
            opened(),
            ChangeEvent::Revised {
                revision: revision(2, 2),
            },
        ])
        .try_when(|change, _| change.merge(1, commit(1)))
        .then_error("change.stale_revision");
    }

    #[test]
    fn events_serialize_stably() {
        let events = vec![
            opened(),
            ChangeEvent::Revised {
                revision: revision(2, 2),
            },
            ChangeEvent::Closed,
            ChangeEvent::Approved {
                approval: Approval {
                    revision: 1,
                    by: Id::from_uuid(Uuid::from_u128(7)),
                    at: S::NOW,
                },
            },
            ChangeEvent::Commented {
                comment: comment(1, "Handle the empty case"),
            },
            ChangeEvent::ChangesRequested {
                request: ChangeRequest {
                    revision: 1,
                    by: comment(1, "").author,
                    comments: vec![comment(1, "Handle the empty case")],
                    at: S::NOW,
                },
            },
            ChangeEvent::Merged {
                revision: 1,
                commit: commit(1),
            },
        ];
        insta::assert_json_snapshot!(events);
    }
}
