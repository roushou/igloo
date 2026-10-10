use std::collections::BTreeMap;
use std::sync::Arc;

use igloo_core::change::{Change, ChangeId, ChangePhase};
use igloo_core::repo::{BranchName, CommitId, Repo};
use igloo_core::{Entity, ErrorCode as _};
use tracing::warn;

use super::{ChangeHeads, ChangeQueries, OpenChange, RepoQueries, ReviseChange};
use crate::app::{AppError, CommandBus, InstallError, PlatformBuilder, RequestContext};
use crate::ports::Forge;

/// The branches of a repository's copy and their heads.
pub type Branches = BTreeMap<BranchName, CommitId>;

/// A change a push opened or revised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushedChange {
    /// The branch pushed.
    pub branch: BranchName,
    /// Its change.
    pub change: ChangeId,
    /// Whether the push opened the change, rather than recording a revision of it.
    pub opened: bool,
}

/// Turns the branches a push moved into changes: a branch with no open change opens one, titled
/// with the subject of the branch's head commit; a branch with an open change records a
/// revision.
///
/// Invariant: the default branch never becomes a change, and a branch with nothing to propose
/// (its head is where it forked from the default branch) opens none. A branch that cannot be
/// proposed is logged and skipped, since the push it came from has already happened.
#[derive(Clone)]
pub struct BranchPushes {
    changes: ChangeQueries,
    heads: ChangeHeads,
    forge: Arc<dyn Forge>,
    bus: CommandBus,
}

impl BranchPushes {
    /// The longest title of a change, in characters.
    const MAX_TITLE: usize = 200;

    /// Records pushes over the stores and forge `platform` was given, dispatching on `bus`.
    pub fn new(platform: &PlatformBuilder, bus: CommandBus) -> Result<Self, InstallError> {
        let ports = platform.ports();
        let repos = RepoQueries::new(platform.store::<Repo>()?, Arc::clone(&ports.secrets));
        Ok(Self {
            changes: ChangeQueries::new(platform.store::<Change>()?),
            heads: ChangeHeads::new(repos, Arc::clone(&ports.forge)),
            forge: Arc::clone(&ports.forge),
            bus,
        })
    }

    /// The branches of `repo`'s copy now.
    pub async fn branches(&self, repo: &Repo) -> Result<Branches, AppError> {
        Ok(self.forge.branches(repo.id()).await?.into_iter().collect())
    }

    /// Opens or revises the changes of the branches that moved from `before` to `after`, as
    /// the sender of `context`. Branches that disappeared, and the default branch, are
    /// ignored.
    pub async fn record(
        &self,
        repo: &Repo,
        before: &Branches,
        after: &Branches,
        context: &RequestContext,
    ) -> Result<Vec<PushedChange>, AppError> {
        let open = self.open_changes(repo).await?;
        let mut recorded = Vec::new();
        for (branch, head) in after {
            if branch == repo.default_branch() || before.get(branch) == Some(head) {
                continue;
            }
            match self.propose(repo, &open, branch, context).await {
                Ok(Some(pushed)) => recorded.push(pushed),
                Ok(None) => {}
                Err(error) => {
                    warn!(repo = %repo.id(), %branch, code = error.code(), %error, "a pushed branch did not become a change");
                }
            }
        }
        Ok(recorded)
    }

    async fn open_changes(&self, repo: &Repo) -> Result<BTreeMap<BranchName, Change>, AppError> {
        Ok(self
            .changes
            .of_repo(repo.id())
            .await?
            .into_iter()
            .filter(|change| *change.phase() == ChangePhase::Open)
            .map(|change| (change.source().clone(), change))
            .collect())
    }

    async fn propose(
        &self,
        repo: &Repo,
        open: &BTreeMap<BranchName, Change>,
        branch: &BranchName,
        context: &RequestContext,
    ) -> Result<Option<PushedChange>, AppError> {
        let heads = self.heads.held(repo, branch).await?;
        if let Some(change) = open.get(branch) {
            let id = change.id();
            self.bus
                .dispatch(ReviseChange { change: id, heads }, context.clone())
                .await?;
            return Ok(Some(PushedChange {
                branch: branch.clone(),
                change: id,
                opened: false,
            }));
        }
        if heads.head == heads.base {
            return Ok(None);
        }
        let title = self.title(repo, branch, &heads.base, &heads.head).await?;
        let command = OpenChange {
            repo: repo.id(),
            source: branch.clone(),
            title,
            heads,
        };
        let change = self.bus.dispatch(command, context.clone()).await?;
        Ok(Some(PushedChange {
            branch: branch.clone(),
            change,
            opened: true,
        }))
    }

    /// The subject of the commit at `head`, or the branch's name when it has none.
    async fn title(
        &self,
        repo: &Repo,
        branch: &BranchName,
        base: &CommitId,
        head: &CommitId,
    ) -> Result<String, AppError> {
        let commits = self.forge.commits(repo.id(), base, head).await?;
        let subject = commits
            .last()
            .and_then(|(_, message)| message.lines().next())
            .map(str::trim)
            .filter(|subject| !subject.is_empty());
        Ok(subject.map_or_else(
            || branch.to_string(),
            |subject| subject.chars().take(Self::MAX_TITLE).collect(),
        ))
    }
}
