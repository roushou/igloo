use std::sync::Arc;

use igloo_core::change::{Change, ChangeError, ChangePhase};
use igloo_core::repo::Repo;
use igloo_core::{Entity, ErrorCode};

use super::commands::RunQueries;
use super::run::RunOutcome;
use crate::app::{AppError, CommandBus, RequestContext};
use crate::platform::{MergeChange, RepoQueries, TrustSettings};
use crate::ports::{Expected, Forge, ForgeError};

/// Merges changes into their target branch by fast-forward, once their latest revision's run
/// passed and, when they touch a protected path of the target branch's trust settings, a human
/// approved that revision.
///
/// Invariant: the target branch moves only from the commit the checks were judged against.
#[derive(Clone)]
pub struct Merger {
    repos: RepoQueries,
    runs: RunQueries,
    forge: Arc<dyn Forge>,
    bus: CommandBus,
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
        let passed = self.runs.of_change(change.id()).await?.iter().any(|run| {
            run.revision() == revision.number && run.outcome() == Some(&RunOutcome::Passed)
        });
        if !passed {
            return Err(AppError::domain(&MergeError::ChecksNotPassed));
        }
        let repo = self
            .repos
            .get(change.repo())
            .await?
            .ok_or_else(|| AppError::not_found(Repo::NAME, &change.repo()))?;
        let remote = self.repos.remote(&repo).await?;
        let target = self.forge.fetch(&remote, change.target()).await?;
        let forked = self
            .forge
            .merge_base(repo.id(), &revision.head, &target)
            .await?;
        if forked.as_ref() != Some(&target) {
            return Err(AppError::domain(&MergeError::TargetMoved));
        }
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
        let protected = trust.protected(&changed);
        if !protected.is_empty() && !change.approved_by_human(revision.number) {
            return Err(AppError::domain(&MergeError::ApprovalRequired(
                protected.join(", "),
            )));
        }
        match self
            .forge
            .push(
                &remote,
                &revision.head,
                change.target(),
                Expected::At(target),
            )
            .await
        {
            Ok(()) => {}
            Err(ForgeError::Moved(_)) => return Err(AppError::domain(&MergeError::TargetMoved)),
            Err(error) => return Err(error.into()),
        }
        let command = MergeChange {
            change: change.id(),
            revision: revision.number,
            commit: revision.head.clone(),
        };
        self.bus.dispatch(command, context).await
    }
}
