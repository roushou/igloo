use std::sync::Arc;

use igloo_core::change::{Change, ChangeError, ChangeId, ChangePhase, Revision};
use igloo_core::repo::{CommitId, Repo};
use igloo_core::{Entity, ErrorCode};

use super::commands::RunQueries;
use super::run::{Run, RunOutcome};
use crate::app::{AppError, CommandBus, RequestContext};
use crate::platform::{MergeChange, RepoQueries, TrustSettings};
use crate::ports::{Expected, Forge, ForgeError, Remote};

/// Merges changes into their target branch as one squashed commit, once their latest revision's
/// run passed, the revision is based on the target's head and, when it touches a protected path
/// of the target branch's trust settings, a human approved that revision.
///
/// Invariant: the target branch moves only from the commit the checks were judged against, to
/// a commit holding exactly the judged revision's tree.
#[derive(Clone)]
pub struct Merger {
    repos: RepoQueries,
    runs: RunQueries,
    forge: Arc<dyn Forge>,
    bus: CommandBus,
}

/// The state of the checks of a change's latest revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksState {
    /// Its run passed.
    Passed,
    /// Its run failed or errored.
    Failed,
    /// Its run has not ended.
    Running,
    /// It has no run.
    Missing,
}

/// What stands between a change and its merge, as of one fetch of the target branch.
///
/// Invariant: `protected` is empty unless `fast_forward` holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Readiness {
    /// The latest revision's checks.
    pub checks: ChecksState,
    /// Whether the latest revision is based on the target branch's head.
    pub fast_forward: bool,
    /// The protected paths the latest revision changes.
    pub protected: Vec<String>,
    /// Whether a human approved the latest revision.
    pub approved: bool,
}

/// The target branch's head and where the latest revision forked from it.
struct Position {
    target: CommitId,
    forked: Option<CommitId>,
    /// The protected paths changed; known only when the revision is based on the head.
    protected: Option<Vec<String>>,
}

impl Position {
    fn protected(&self) -> Option<&[String]> {
        self.protected.as_deref()
    }
}

/// Why a change cannot merge.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MergeError {
    /// The latest revision's run has not passed.
    #[error("the latest revision's checks have not passed")]
    ChecksNotPassed,
    /// The target branch moved since the revision forked; a rebased revision is needed.
    #[error("the target branch moved; push a revision based on its head")]
    TargetMoved,
    /// The change touches protected paths and no human approved the latest revision.
    #[error("protected paths changed without a human approval: {0}")]
    ApprovalRequired(String),
}

impl ErrorCode for MergeError {
    fn code(&self) -> &'static str {
        match self {
            Self::ChecksNotPassed => "change.checks_not_passed",
            Self::TargetMoved => "change.target_moved",
            Self::ApprovalRequired(_) => "change.approval_required",
        }
    }
}

impl Merger {
    /// A merger reaching forges through `repos`, judging checks with `runs`.
    #[must_use]
    pub fn new(
        repos: RepoQueries,
        runs: RunQueries,
        forge: Arc<dyn Forge>,
        bus: CommandBus,
    ) -> Self {
        Self {
            repos,
            runs,
            forge,
            bus,
        }
    }

    /// Merges `change` on behalf of `context`. A merged change merges again as a no-op.
    pub async fn merge(&self, change: &Change, context: RequestContext) -> Result<(), AppError> {
        match change.phase() {
            ChangePhase::Open => {}
            ChangePhase::Merged { .. } => return Ok(()),
            ChangePhase::Closed => return Err(AppError::domain(&ChangeError::Ended)),
        }
        let revision = change.latest();
        if self.checks(change).await? != ChecksState::Passed {
            return Err(AppError::domain(&MergeError::ChecksNotPassed));
        }
        let remote = self.remote(change).await?;
        let target = self.forge.fetch(&remote, change.target()).await?;
        let position = self.position(change, target).await?;
        let Some(protected) = position.protected() else {
            // An earlier attempt may have pushed the squash and stopped before recording it.
            if let Some(base) = position.forked
                && self.squash(change, revision, &base).await? == position.target
            {
                return self
                    .record(change, revision, position.target, context)
                    .await;
            }
            return Err(AppError::domain(&MergeError::TargetMoved));
        };
        if !protected.is_empty() && !change.approved_by_human(revision.number) {
            return Err(AppError::domain(&MergeError::ApprovalRequired(
                protected.join(", "),
            )));
        }
        let squash = self.squash(change, revision, &position.target).await?;
        match self
            .forge
            .advance(
                change.repo(),
                change.target(),
                &squash,
                Expected::At(position.target),
            )
            .await
        {
            Ok(()) => {}
            Err(ForgeError::Moved(_)) => return Err(AppError::domain(&MergeError::TargetMoved)),
            Err(error) => return Err(error.into()),
        }
        self.record(change, revision, squash, context).await
    }

    /// What stands between `change`'s latest revision and its merge, by the rules
    /// [`Merger::merge`] applies, judged against the target branch's head in the mirror as of
    /// its last fetch. Never reaches the forge; a mirror without the target reports no fast
    /// forward and no protected paths.
    pub async fn readiness(&self, change: &Change) -> Result<Readiness, AppError> {
        let checks = self.checks(change).await?;
        let revision = change.latest().number;
        let protected = match self.forge.mirrored(change.repo(), change.target()).await? {
            Some(target) => self.position(change, target).await?.protected,
            None => None,
        };
        Ok(Readiness {
            checks,
            fast_forward: protected.is_some(),
            approved: change.approved_by_human(revision),
            protected: protected.unwrap_or_default(),
        })
    }

    /// The state of the latest revision's run.
    async fn checks(&self, change: &Change) -> Result<ChecksState, AppError> {
        let number = change.latest().number;
        let runs = self.runs.of_change(change.id()).await?;
        let outcomes: Vec<Option<&RunOutcome>> = runs
            .iter()
            .filter(|run| run.revision() == number)
            .map(Run::outcome)
            .collect();
        Ok(if outcomes.contains(&Some(&RunOutcome::Passed)) {
            ChecksState::Passed
        } else if outcomes.iter().any(Option::is_some) {
            ChecksState::Failed
        } else if outcomes.is_empty() {
            ChecksState::Missing
        } else {
            ChecksState::Running
        })
    }

    /// Where the latest revision sits against `target`, the target branch's head in the mirror.
    async fn position(&self, change: &Change, target: CommitId) -> Result<Position, AppError> {
        let revision = change.latest();
        let repo = self.repo(change).await?;
        let forked = self
            .forge
            .merge_base(repo.id(), &revision.head, &target)
            .await?;
        let protected = if forked.as_ref() == Some(&target) {
            let trust = match self
                .forge
                .read_file(repo.id(), &target, TrustSettings::PATH)
                .await?
            {
                Some(bytes) => TrustSettings::try_from(String::from_utf8_lossy(&bytes).as_ref())
                    .map_err(AppError::Validation)?,
                None => TrustSettings::default(),
            };
            let changed = self
                .forge
                .changed_paths(repo.id(), &target, &revision.head)
                .await?;
            Some(
                trust
                    .protected(&changed)
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            )
        } else {
            None
        };
        Ok(Position {
            target,
            forked,
            protected,
        })
    }

    async fn repo(&self, change: &Change) -> Result<Repo, AppError> {
        self.repos
            .get(change.repo())
            .await?
            .ok_or_else(|| AppError::not_found(Repo::NAME, &change.repo()))
    }

    async fn remote(&self, change: &Change) -> Result<Remote, AppError> {
        let repo = self.repo(change).await?;
        self.repos.remote(&repo).await
    }

    /// The squash of `revision` onto `onto`: the same commit for the same change, revision and
    /// base, so a retried merge recognizes the one it pushed.
    async fn squash(
        &self,
        change: &Change,
        revision: &Revision,
        onto: &CommitId,
    ) -> Result<CommitId, AppError> {
        let commits = self
            .forge
            .commits(change.repo(), onto, &revision.head)
            .await?;
        let message = SquashMessage::new(change.title(), change.id(), &commits);
        Ok(self
            .forge
            .squash(
                change.repo(),
                onto,
                &revision.head,
                message.as_str(),
                revision.at,
            )
            .await?)
    }

    async fn record(
        &self,
        change: &Change,
        revision: &Revision,
        commit: CommitId,
        context: RequestContext,
    ) -> Result<(), AppError> {
        let command = MergeChange {
            change: change.id(),
            revision: revision.number,
            commit,
        };
        self.bus.dispatch(command, context).await
    }
}

/// The message of a change's squashed commit. One commit keeps its message; several are
/// summed up under the change's title, one line each. Either way the trailers of the commits
/// follow once each, then `Igloo-Change` naming the change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SquashMessage(String);

impl SquashMessage {
    /// The trailer naming the change.
    pub(super) const CHANGE: &'static str = "Igloo-Change";

    /// The message squashing `commits`, oldest first with their messages, of the change
    /// `change` titled `title`.
    #[must_use]
    pub(super) fn new(title: &str, change: ChangeId, commits: &[(CommitId, String)]) -> Self {
        let mut trailers: Vec<&str> = Vec::new();
        let mut bodies: Vec<&str> = Vec::new();
        for (_, message) in commits {
            let (body, own) = Self::split(message);
            bodies.push(body);
            for trailer in own {
                if !trailers.contains(&trailer) {
                    trailers.push(trailer);
                }
            }
        }
        let mut text = if let [only] = bodies.as_slice() {
            (*only).to_owned()
        } else {
            let lines: Vec<String> = bodies
                .iter()
                .map(|body| format!("* {}", body.lines().next().unwrap_or_default()))
                .collect();
            format!("{title}\n\n{}", lines.join("\n"))
        };
        let own = format!("{}: {change}", Self::CHANGE);
        text.push_str("\n\n");
        for trailer in trailers.iter().filter(|trailer| **trailer != own) {
            text.push_str(trailer);
            text.push('\n');
        }
        text.push_str(&own);
        Self(text)
    }

    /// The message.
    #[must_use]
    pub(super) fn as_str(&self) -> &str {
        &self.0
    }

    /// `message` without its trailer block, and the trailers: the lines of its last paragraph
    /// when every one reads `Key: value`.
    fn split(message: &str) -> (&str, Vec<&str>) {
        let message = message.trim_end();
        let (body, last) = message
            .rsplit_once("\n\n")
            .map_or(("", message), |(body, last)| (body, last));
        let is_trailer = |line: &str| {
            line.split_once(": ").is_some_and(|(key, _)| {
                !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
        };
        if !body.is_empty() && last.lines().all(is_trailer) {
            (body.trim_end(), last.lines().collect())
        } else {
            (message, Vec::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use igloo_core::Id;
    use uuid::Uuid;

    use super::*;

    fn commits(messages: &[&str]) -> Vec<(CommitId, String)> {
        messages
            .iter()
            .enumerate()
            .map(|(n, message)| {
                (
                    format!("{n:040x}").parse().expect("commit"),
                    (*message).to_owned(),
                )
            })
            .collect()
    }

    fn change() -> ChangeId {
        Id::from_uuid(Uuid::from_u128(1))
    }

    #[test]
    fn one_commit_keeps_its_message_and_names_the_change() {
        let message = SquashMessage::new(
            "Title",
            change(),
            &commits(&["fix: a thing\n\nWhy.\n\nIgloo-Task: task_1"]),
        );
        assert_eq!(
            message.as_str(),
            format!(
                "fix: a thing\n\nWhy.\n\nIgloo-Task: task_1\nIgloo-Change: {}",
                change()
            )
        );
    }

    #[test]
    fn several_commits_are_listed_under_the_title_with_their_trailers_once() {
        let message = SquashMessage::new(
            "Event stream",
            change(),
            &commits(&[
                "feat(api): stream events\n\nBody.\n\nCo-Authored-By: A <a@x>\nIgloo-Task: task_1",
                "feat(web): follow events\n\nIgloo-Task: task_1",
                "chore: tidy",
            ]),
        );
        assert_eq!(
            message.as_str(),
            format!(
                "Event stream\n\n* feat(api): stream events\n* feat(web): follow events\n* \
                 chore: tidy\n\nCo-Authored-By: A <a@x>\nIgloo-Task: task_1\nIgloo-Change: {}",
                change()
            )
        );
    }

    #[test]
    fn a_paragraph_that_is_not_all_trailers_stays_in_the_body() {
        let message = SquashMessage::new(
            "Title",
            change(),
            &commits(&["fix: x\n\nNote: this is prose\nand more prose"]),
        );
        assert_eq!(
            message.as_str(),
            format!(
                "fix: x\n\nNote: this is prose\nand more prose\n\nIgloo-Change: {}",
                change()
            )
        );
    }
}
