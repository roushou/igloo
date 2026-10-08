use igloo_core::change::{Approval, ChangeId};
use igloo_core::repo::{CommitId, RepoId};
use igloo_core::{Entity, Event, Id, Prefixed, Timestamp};
use serde::{Deserialize, Serialize};

use super::run::CheckOutcome;

/// Identifies an outcome (`out_...`).
pub type OutcomeId = Id<Outcome>;

/// How a change ended, kept for every repository as the evidence autonomy is earned from: the
/// verdict, the checks of the revision judged, the approvals, and whether a merge was reverted.
///
/// Invariant: one per change; recorded once, then only the agent's work and a revert are added,
/// once each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    id: OutcomeId,
    record: OutcomeRecord,
    agent: Option<AgentWork>,
    reverted_by: Option<CommitId>,
    events: Vec<OutcomeEvent>,
}

/// What an outcome records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeRecord {
    /// The repository.
    pub repo: RepoId,
    /// The change.
    pub change: ChangeId,
    /// How it ended.
    pub verdict: Verdict,
    /// The revision judged: the merged one, or the latest when closed.
    pub revision: u32,
    /// The checks of that revision's run, in pipeline order.
    pub checks: Vec<CheckResult>,
    /// The approvals of that revision.
    pub approvals: Vec<Approval>,
    /// The commits the change brought to its target branch, oldest first; empty when closed.
    pub commits: Vec<CommitId>,
    /// When the change ended.
    pub at: Timestamp,
}

/// The agent's work behind a change made by a task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWork {
    /// The task (`task_...`).
    pub task: String,
    /// The tool, as named in the repository's settings.
    pub tool: String,
    /// How many turns the tool took.
    pub turns: u32,
}

/// How a change ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// Merged: the target branch points at `commit`.
    Merged {
        /// The target branch's head after the merge.
        commit: CommitId,
    },
    /// Closed without merging.
    Closed,
}

/// One check's result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    /// The check's name.
    pub name: String,
    /// How it ended; none when it never ran.
    pub outcome: Option<CheckOutcome>,
}

/// Facts about an outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutcomeEvent {
    /// A change ended.
    Recorded {
        /// Its id.
        id: OutcomeId,
        /// What it records.
        record: OutcomeRecord,
    },
    /// The change was made by a task's agent.
    Attributed {
        /// The agent's work.
        work: AgentWork,
    },
    /// A commit on the target branch reverted one of the merged commits.
    Reverted {
        /// The reverting commit.
        commit: CommitId,
    },
}

impl Outcome {
    /// Records how a change ended.
    #[must_use]
    pub fn new(id: OutcomeId, record: OutcomeRecord) -> Self {
        let mut outcome = Self::initial(id, record.clone());
        outcome.events.push(OutcomeEvent::Recorded { id, record });
        outcome
    }

    /// Records that `commit` reverted the merge. Only the first revert of a merged change is
    /// recorded.
    pub fn revert(&mut self, commit: CommitId) {
        if self.reverted_by.is_none() && matches!(self.record.verdict, Verdict::Merged { .. }) {
            self.apply_and_push(OutcomeEvent::Reverted { commit });
        }
    }

    /// Records the agent's work behind the change; only the first attribution counts.
    pub fn attribute(&mut self, work: AgentWork) {
        if self.agent.is_none() {
            self.apply_and_push(OutcomeEvent::Attributed { work });
        }
    }

    /// The agent's work behind the change, if a task made it.
    #[must_use]
    pub const fn agent(&self) -> Option<&AgentWork> {
        self.agent.as_ref()
    }

    /// What it records.
    #[must_use]
    pub const fn record(&self) -> &OutcomeRecord {
        &self.record
    }

    /// The commit that reverted the merge, if one did.
    #[must_use]
    pub const fn reverted_by(&self) -> Option<&CommitId> {
        self.reverted_by.as_ref()
    }

    /// The merged commit `message` reverts, as `git revert` words it.
    #[must_use]
    pub fn reverted_in(&self, message: &str) -> bool {
        self.record
            .commits
            .iter()
            .any(|commit| message.contains(&format!("This reverts commit {commit}")))
    }

    const fn initial(id: OutcomeId, record: OutcomeRecord) -> Self {
        Self {
            id,
            record,
            agent: None,
            reverted_by: None,
            events: Vec::new(),
        }
    }

    fn apply_and_push(&mut self, event: OutcomeEvent) {
        self.apply(&event);
        self.events.push(event);
    }
}

impl Prefixed for Outcome {
    const PREFIX: &'static str = "out";
}

impl Event for OutcomeEvent {
    const SCHEMA_VERSION: u16 = 1;

    fn kind(&self) -> &'static str {
        match self {
            Self::Recorded { .. } => "igloo.outcome.recorded",
            Self::Attributed { .. } => "igloo.outcome.attributed",
            Self::Reverted { .. } => "igloo.outcome.reverted",
        }
    }
}

impl Entity for Outcome {
    const NAME: &'static str = "outcome";

    type Event = OutcomeEvent;

    fn id(&self) -> OutcomeId {
        self.id
    }

    fn from_created(event: &OutcomeEvent) -> Option<Self> {
        let OutcomeEvent::Recorded { id, record } = event else {
            return None;
        };
        Some(Self::initial(*id, record.clone()))
    }

    fn apply(&mut self, event: &OutcomeEvent) {
        match event {
            OutcomeEvent::Recorded { .. } => {}
            OutcomeEvent::Attributed { work } => self.agent = Some(work.clone()),
            OutcomeEvent::Reverted { commit } => self.reverted_by = Some(commit.clone()),
        }
    }

    fn take_events(&mut self) -> Vec<OutcomeEvent> {
        std::mem::take(&mut self.events)
    }
}

#[cfg(test)]
mod tests {
    use igloo_core::testing::Scenario;
    use uuid::Uuid;

    use super::*;

    type S = Scenario<Outcome>;

    fn commit(n: u8) -> CommitId {
        format!("{n:040x}").parse().expect("commit")
    }

    fn record(verdict: Verdict) -> OutcomeRecord {
        OutcomeRecord {
            repo: Id::from_uuid(Uuid::from_u128(1)),
            change: Id::from_uuid(Uuid::from_u128(2)),
            verdict,
            revision: 2,
            checks: vec![CheckResult {
                name: "test".to_owned(),
                outcome: Some(CheckOutcome::Passed),
            }],
            approvals: Vec::new(),
            commits: vec![commit(1), commit(2)],
            at: S::NOW,
        }
    }

    fn merged() -> OutcomeEvent {
        OutcomeEvent::Recorded {
            id: S::ID,
            record: record(Verdict::Merged { commit: commit(2) }),
        }
    }

    #[test]
    fn a_merge_is_reverted_once_by_a_commit_reverting_one_of_its_commits() {
        let scenario = S::given([merged()]);
        let message = format!("Revert \"x\"\n\nThis reverts commit {}.", commit(1));
        assert!(scenario.state().reverted_in(&message));
        assert!(!scenario.state().reverted_in("an unrelated change"));
        scenario
            .when(|outcome, _| outcome.revert(commit(9)))
            .then([OutcomeEvent::Reverted { commit: commit(9) }])
            .when(|outcome, _| outcome.revert(commit(8)))
            .then_no_events();
    }

    #[test]
    fn an_outcome_names_the_agent_work_behind_it_once() {
        let work = |turns| AgentWork {
            task: "task_1".to_owned(),
            tool: "claude-code".to_owned(),
            turns,
        };
        let scenario = S::given([merged()])
            .when(|outcome, _| outcome.attribute(work(2)))
            .then([OutcomeEvent::Attributed { work: work(2) }])
            .when(|outcome, _| outcome.attribute(work(3)))
            .then_no_events();
        assert_eq!(scenario.state().agent(), Some(&work(2)));
    }

    #[test]
    fn a_closed_change_is_never_reverted() {
        S::given([OutcomeEvent::Recorded {
            id: S::ID,
            record: record(Verdict::Closed),
        }])
        .when(|outcome, _| outcome.revert(commit(9)))
        .then_no_events();
    }

    #[test]
    fn events_serialize_stably() {
        insta::assert_json_snapshot!(vec![merged(), OutcomeEvent::Reverted { commit: commit(9) },]);
    }
}
