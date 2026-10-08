use std::sync::Arc;

use async_trait::async_trait;
use igloo_core::change::{Change, ChangeEvent, ChangeId, ChangePhase};
use igloo_core::repo::{CommitId, RepoId};
use igloo_core::{Actor, Entity, SystemComponent};

use super::commands::RunQueries;
use super::outcome::{AgentWork, CheckResult, Outcome, OutcomeId, OutcomeRecord, Verdict};
use crate::app::{AppError, Command, CommandBus, CommandHandler, Reactor, RequestContext};
use crate::platform::{ChangeQueries, RepoQueries};
use crate::ports::{
    Clock, EntityStore, EventEnvelope, Forge, IdGenerator, IdGeneratorExt, StorageError, Versioned,
};

/// Records how a change ended, unless it already is.
pub struct RecordOutcome {
    /// What to record.
    pub record: OutcomeRecord,
}

/// Records that a commit reverted a merged change.
pub struct RecordRevert {
    /// The merged change's outcome.
    pub outcome: OutcomeId,
    /// The reverting commit.
    pub commit: CommitId,
}

impl Command for RecordOutcome {
    type Output = OutcomeId;
    const NAME: &'static str = "outcome.record";
}

/// Records the agent's work behind a change's outcome.
pub struct AttributeOutcome {
    /// The outcome.
    pub outcome: OutcomeId,
    /// The agent's work.
    pub work: AgentWork,
}

impl Command for AttributeOutcome {
    type Output = ();
    const NAME: &'static str = "outcome.attribute";
}

impl Command for RecordRevert {
    type Output = ();
    const NAME: &'static str = "outcome.record_revert";
}

/// Read access to outcomes.
#[derive(Clone)]
pub struct OutcomeQueries {
    store: Arc<dyn EntityStore<Outcome>>,
}

impl OutcomeQueries {
    /// Queries over `store`.
    #[must_use]
    pub fn new(store: Arc<dyn EntityStore<Outcome>>) -> Self {
        Self { store }
    }

    /// The outcome of `change`, once it ended.
    pub async fn of_change(&self, change: ChangeId) -> Result<Option<Outcome>, StorageError> {
        Ok(self
            .store
            .all()
            .await?
            .into_iter()
            .find(|outcome| outcome.record().change == change))
    }

    /// The merged, unreverted changes of `repo`.
    pub async fn standing_merges(&self, repo: RepoId) -> Result<Vec<Outcome>, StorageError> {
        Ok(self
            .store
            .all()
            .await?
            .into_iter()
            .filter(|outcome| {
                outcome.record().repo == repo
                    && matches!(outcome.record().verdict, Verdict::Merged { .. })
                    && outcome.reverted_by().is_none()
            })
            .collect())
    }
}

/// Records an outcome once per change.
pub(super) struct RecordOutcomeHandler {
    pub(super) queries: OutcomeQueries,
    pub(super) store: Arc<dyn EntityStore<Outcome>>,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) ids: Arc<dyn IdGenerator>,
}

#[async_trait]
impl CommandHandler<RecordOutcome> for RecordOutcomeHandler {
    async fn handle(
        &self,
        command: RecordOutcome,
        context: &RequestContext,
    ) -> Result<OutcomeId, AppError> {
        if let Some(existing) = self.queries.of_change(command.record.change).await? {
            return Ok(existing.id());
        }
        let id = self.ids.next::<Outcome>();
        let mut outcome = Versioned::new(Outcome::new(id, command.record));
        self.store
            .commit(&mut outcome, &context.commit_meta(self.clock.now()))
            .await?;
        Ok(id)
    }
}

/// Records the outcome of every change that ends.
pub(super) struct RecordOutcomes {
    pub(super) changes: ChangeQueries,
    pub(super) runs: RunQueries,
    pub(super) forge: Arc<dyn Forge>,
}

/// Records reverts of merged changes found on default branches, looking whenever a change
/// event shows a fresh fetch of one.
pub(super) struct RecordReverts {
    pub(super) changes: ChangeQueries,
    pub(super) repos: RepoQueries,
    pub(super) outcomes: OutcomeQueries,
    pub(super) forge: Arc<dyn Forge>,
}

fn context(event: &EventEnvelope) -> RequestContext {
    RequestContext::caused_by(
        event,
        Actor::System {
            component: SystemComponent::Reactor,
        },
    )
}

impl RecordOutcomes {
    async fn record(&self, change: &Change) -> Result<OutcomeRecord, AppError> {
        let (verdict, revision) = match change.phase() {
            ChangePhase::Merged { revision, commit } => (
                Verdict::Merged {
                    commit: commit.clone(),
                },
                *revision,
            ),
            ChangePhase::Closed | ChangePhase::Open => (Verdict::Closed, change.latest().number),
        };
        let checks = self
            .runs
            .of_change(change.id())
            .await?
            .into_iter()
            .find(|run| run.revision() == revision)
            .map(|run| {
                run.checks()
                    .iter()
                    .map(|check| CheckResult {
                        name: check.spec.name.clone(),
                        outcome: check.outcome.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let judged = change
            .revisions()
            .find(|candidate| candidate.number == revision)
            .unwrap_or_else(|| change.latest());
        let commits = match verdict {
            Verdict::Merged { .. } => self
                .forge
                .commits(change.repo(), &judged.base, &judged.head)
                .await?
                .into_iter()
                .map(|(commit, _)| commit)
                .collect(),
            Verdict::Closed => Vec::new(),
        };
        Ok(OutcomeRecord {
            repo: change.repo(),
            change: change.id(),
            verdict,
            revision,
            checks,
            approvals: change
                .approvals()
                .iter()
                .filter(|approval| approval.revision == revision)
                .cloned()
                .collect(),
            commits,
            at: judged.at,
        })
    }
}

#[async_trait]
impl Reactor for RecordOutcomes {
    fn name(&self) -> &'static str {
        "ci.record_outcomes"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let Some(id) = event.subject_as::<Change>() else {
            return Ok(());
        };
        if !matches!(
            event.decode::<ChangeEvent>()?,
            ChangeEvent::Merged { .. } | ChangeEvent::Closed
        ) {
            return Ok(());
        }
        let Some(change) = self.changes.get(id).await? else {
            return Ok(());
        };
        let mut record = self.record(&change).await?;
        record.at = event.time;
        bus.dispatch(RecordOutcome { record }, context(event))
            .await
            .map(drop)
    }
}

#[async_trait]
impl Reactor for RecordReverts {
    fn name(&self) -> &'static str {
        "ci.record_reverts"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let Some(change) = event.subject_as::<Change>() else {
            return Ok(());
        };
        if matches!(
            event.decode::<ChangeEvent>()?,
            ChangeEvent::Closed | ChangeEvent::Approved { .. }
        ) {
            return Ok(());
        }
        let Some(repo) = self.changes.get(change).await?.map(|change| change.repo()) else {
            return Ok(());
        };
        let merges = self.outcomes.standing_merges(repo).await?;
        if merges.is_empty() {
            return Ok(());
        }
        let Some(found) = self.repos.get(repo).await? else {
            return Ok(());
        };
        let remote = self.repos.remote(&found).await?;
        let head = self.forge.fetch(&remote, found.default_branch()).await?;
        for outcome in merges {
            let Verdict::Merged { commit } = &outcome.record().verdict else {
                continue;
            };
            let later = self.forge.commits(repo, commit, &head).await?;
            if let Some((revert, _)) = later
                .into_iter()
                .find(|(_, message)| outcome.reverted_in(message))
            {
                let command = RecordRevert {
                    outcome: outcome.id(),
                    commit: revert,
                };
                bus.dispatch(command, context(event)).await?;
            }
        }
        Ok(())
    }
}
