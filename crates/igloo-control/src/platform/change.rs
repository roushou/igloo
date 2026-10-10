use std::sync::Arc;

use async_trait::async_trait;
use igloo_core::change::{Change, ChangeError, ChangeEvent, ChangeId, ChangePhase, Comment};
use igloo_core::repo::{BranchName, CommitId, Repo, RepoId};
use igloo_core::{Actor, Entity, ErrorCode, Timestamp};
use tracing::info;

use super::RepoQueries;
use crate::app::{
    AppError, Command, CommandBus, CommandHandler, EntityHandler, Extension, InstallError,
    PlatformBuilder, Reactor, RequestContext,
};
use crate::ports::{
    Clock, EntityStore, EventEnvelope, Forge, ForgeError, IdGenerator, IdGeneratorExt,
    StorageError, Versioned,
};

/// Opens a change proposing `source`, whose head and fork point were just fetched.
pub struct OpenChange {
    /// The repository.
    pub repo: RepoId,
    /// The branch proposed.
    pub source: BranchName,
    /// Its title.
    pub title: String,
    /// The first revision's head and base, from [`ChangeHeads::of`].
    pub heads: Heads,
}

/// Records a change's source head as its next revision.
pub struct ReviseChange {
    /// The change.
    pub change: ChangeId,
    /// The head and base, from [`ChangeHeads::of`].
    pub heads: Heads,
}

/// Closes a change without merging.
pub struct CloseChange {
    /// The change.
    pub change: ChangeId,
}

/// Approves a change's latest revision as the human sending the command.
pub struct ApproveChange {
    /// The change.
    pub change: ChangeId,
    /// The revision approved; it must be the latest.
    pub revision: u32,
}

/// Comments on a change as the human or agent sending the command.
pub struct CommentChange {
    /// The change.
    pub change: ChangeId,
    /// The revision; the latest when absent.
    pub revision: Option<u32>,
    /// What it says.
    pub body: String,
    /// The file it is about.
    pub path: Option<String>,
    /// The line of `path` it is about, from 1.
    pub line: Option<u32>,
}

/// Asks for another revision of a change, carrying the comments since the previous request.
pub struct RequestChanges {
    /// The change.
    pub change: ChangeId,
}

/// Records that a change's revision was merged: the target branch points at `commit`.
pub struct MergeChange {
    /// The change.
    pub change: ChangeId,
    /// The revision merged.
    pub revision: u32,
    /// The target branch's new head.
    pub commit: CommitId,
}

impl Command for ApproveChange {
    type Output = ();
    const NAME: &'static str = "change.approve";
}

impl Command for CommentChange {
    type Output = ();
    const NAME: &'static str = "change.comment";
}

impl Command for RequestChanges {
    type Output = ();
    const NAME: &'static str = "change.request_changes";
}

impl Command for MergeChange {
    type Output = ();
    const NAME: &'static str = "change.merge";
}

/// A source branch's head and its merge base with the target branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heads {
    /// The source branch's head.
    pub head: CommitId,
    /// Where it forked from the target branch.
    pub base: CommitId,
}

impl Command for OpenChange {
    type Output = ChangeId;
    const NAME: &'static str = "change.open";
}

impl Command for ReviseChange {
    type Output = u32;
    const NAME: &'static str = "change.revise";
}

impl Command for CloseChange {
    type Output = ();
    const NAME: &'static str = "change.close";
}

/// Why a change cannot be opened or revised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ChangeHeadError {
    /// Another open change proposes the same branch.
    #[error("an open change already proposes this branch")]
    AlreadyOpen,
    /// The source branch shares no history with the target branch.
    #[error("the branch shares no history with the target branch")]
    Unrelated,
}

impl ErrorCode for ChangeHeadError {
    fn code(&self) -> &'static str {
        match self {
            Self::AlreadyOpen => "change.already_open",
            Self::Unrelated => "change.unrelated_history",
        }
    }
}

/// Fetches a change's branches and finds where the source forked from the target.
#[derive(Clone)]
pub struct ChangeHeads {
    repos: RepoQueries,
    forge: Arc<dyn Forge>,
}

impl ChangeHeads {
    /// Heads fetched through `forge`, reaching repositories through `repos`.
    #[must_use]
    pub fn new(repos: RepoQueries, forge: Arc<dyn Forge>) -> Self {
        Self { repos, forge }
    }

    /// Reads `source` and `repo`'s default branch from Igloo's copy, without reaching the forge;
    /// returns `source`'s head and its merge base with the default branch.
    pub async fn held(&self, repo: &Repo, source: &BranchName) -> Result<Heads, AppError> {
        let missing =
            |branch: &BranchName| AppError::from(ForgeError::BranchNotFound(branch.clone()));
        let head = self
            .forge
            .mirrored(repo.id(), source)
            .await?
            .ok_or_else(|| missing(source))?;
        let target = self
            .forge
            .mirrored(repo.id(), repo.default_branch())
            .await?
            .ok_or_else(|| missing(repo.default_branch()))?;
        let base = self
            .forge
            .merge_base(repo.id(), &head, &target)
            .await?
            .ok_or_else(|| AppError::domain(&ChangeHeadError::Unrelated))?;
        Ok(Heads { head, base })
    }

    /// Adopts the forge's `source`, which it is the source of, and fetches `repo`'s default
    /// branch, which Igloo's copy wins; returns `source`'s head and its merge base
    /// with the default branch.
    pub async fn of(&self, repo: &Repo, source: &BranchName) -> Result<Heads, AppError> {
        let remote = self.repos.remote(repo).await?;
        let head = self.forge.adopt(&remote, source).await?;
        let target = self.forge.fetch(&remote, repo.default_branch()).await?;
        let base = self
            .forge
            .merge_base(repo.id(), &head, &target)
            .await?
            .ok_or_else(|| AppError::domain(&ChangeHeadError::Unrelated))?;
        Ok(Heads { head, base })
    }
}

/// Read access to changes beyond loading one by id.
#[derive(Clone)]
pub struct ChangeQueries {
    store: Arc<dyn EntityStore<Change>>,
}

impl ChangeQueries {
    /// Queries over `store`.
    #[must_use]
    pub fn new(store: Arc<dyn EntityStore<Change>>) -> Self {
        Self { store }
    }

    /// Change `id`, if any.
    pub async fn get(&self, id: ChangeId) -> Result<Option<Change>, StorageError> {
        Ok(self.store.load(id).await?.map(Versioned::into_inner))
    }

    /// The changes of `repo`, oldest first.
    pub async fn of_repo(&self, repo: RepoId) -> Result<Vec<Change>, StorageError> {
        let mut changes: Vec<Change> = self
            .store
            .all()
            .await?
            .into_iter()
            .filter(|change| change.repo() == repo)
            .collect();
        changes.sort_by_key(Entity::id);
        Ok(changes)
    }
}

/// Opens a change unless another open change proposes the same branch.
struct OpenChangeHandler {
    queries: ChangeQueries,
    repos: RepoQueries,
    store: Arc<dyn EntityStore<Change>>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
}

#[async_trait]
impl CommandHandler<OpenChange> for OpenChangeHandler {
    async fn handle(
        &self,
        command: OpenChange,
        context: &RequestContext,
    ) -> Result<ChangeId, AppError> {
        let repo = self
            .repos
            .get(command.repo)
            .await?
            .ok_or_else(|| AppError::not_found(Repo::NAME, &command.repo))?;
        let taken = self
            .queries
            .of_repo(command.repo)
            .await?
            .iter()
            .any(|change| {
                *change.phase() == ChangePhase::Open && *change.source() == command.source
            });
        if taken {
            return Err(AppError::domain(&ChangeHeadError::AlreadyOpen));
        }
        let id = self.ids.next::<Change>();
        let now = self.clock.now();
        let change = Change::open(
            id,
            command.repo,
            command.source,
            repo.default_branch().clone(),
            &command.title,
            (command.heads.head, command.heads.base),
            now,
        )
        .map_err(|error| AppError::domain(&error))?;
        self.store
            .commit(&mut Versioned::new(change), &context.commit_meta(now))
            .await?;
        Ok(id)
    }
}

/// Approves a revision on behalf of the human sending the command.
#[derive(Clone)]
struct ReviewHandler {
    store: Arc<dyn EntityStore<Change>>,
    clock: Arc<dyn Clock>,
}

impl ReviewHandler {
    /// Applies `review` to change `id` as the sender of the command.
    async fn review(
        &self,
        id: ChangeId,
        context: &RequestContext,
        review: impl FnOnce(&mut Change, Actor, Timestamp) -> Result<(), ChangeError> + Send,
    ) -> Result<(), AppError> {
        let mut change = self
            .store
            .load(id)
            .await?
            .ok_or_else(|| AppError::not_found(Change::NAME, &id))?;
        let now = self.clock.now();
        review(change.entity_mut(), context.actor, now)
            .map_err(|error| AppError::domain(&error))?;
        self.store
            .commit(&mut change, &context.commit_meta(now))
            .await?;
        Ok(())
    }
}

#[async_trait]
impl CommandHandler<ApproveChange> for ReviewHandler {
    async fn handle(
        &self,
        command: ApproveChange,
        context: &RequestContext,
    ) -> Result<(), AppError> {
        self.review(command.change, context, |change, actor, now| {
            change.approve(command.revision, actor, now)
        })
        .await
    }
}

#[async_trait]
impl CommandHandler<CommentChange> for ReviewHandler {
    async fn handle(
        &self,
        command: CommentChange,
        context: &RequestContext,
    ) -> Result<(), AppError> {
        self.review(command.change, context, |change, author, at| {
            change.comment(Comment {
                revision: command.revision.unwrap_or(change.latest().number),
                author,
                body: command.body,
                path: command.path,
                line: command.line,
                at,
            })
        })
        .await
    }
}

#[async_trait]
impl CommandHandler<RequestChanges> for ReviewHandler {
    async fn handle(
        &self,
        command: RequestChanges,
        context: &RequestContext,
    ) -> Result<(), AppError> {
        self.review(command.change, context, Change::request_changes)
            .await
    }
}

/// Changes: proposals to merge a branch, revised as the branch moves.
pub struct ChangeModule;

impl Extension for ChangeModule {
    fn name(&self) -> &'static str {
        "change"
    }

    fn install(self: Box<Self>, platform: &mut PlatformBuilder) -> Result<(), InstallError> {
        let store = platform.store::<Change>()?;
        let ports = platform.ports().clone();
        platform.command(OpenChangeHandler {
            queries: ChangeQueries::new(Arc::clone(&store)),
            repos: RepoQueries::new(platform.store::<Repo>()?, Arc::clone(&ports.secrets)),
            store: Arc::clone(&store),
            clock: Arc::clone(&ports.clock),
            ids: Arc::clone(&ports.ids),
        })?;
        platform.command(EntityHandler::new(
            Arc::clone(&store),
            Arc::clone(&ports.clock),
            |command: &ReviseChange| command.change,
            |change: &mut Change, command: &ReviseChange, now| -> Result<u32, ChangeError> {
                change.revise(command.heads.head.clone(), command.heads.base.clone(), now)
            },
        ))?;
        let review = ReviewHandler {
            store: Arc::clone(&store),
            clock: Arc::clone(&ports.clock),
        };
        platform.command::<ApproveChange>(review.clone())?;
        platform.command::<CommentChange>(review.clone())?;
        platform.command::<RequestChanges>(review)?;
        platform.command(EntityHandler::new(
            Arc::clone(&store),
            Arc::clone(&ports.clock),
            |command: &MergeChange| command.change,
            |change: &mut Change, command: &MergeChange, _| -> Result<(), ChangeError> {
                change.merge(command.revision, command.commit.clone())
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(&store),
            Arc::clone(&ports.clock),
            |command: &CloseChange| command.change,
            |change: &mut Change, _: &CloseChange, _| -> Result<(), ChangeError> { change.close() },
        ))?;
        platform.reactor(DeleteEndedBranches {
            changes: ChangeQueries::new(store),
            repos: RepoQueries::new(platform.store::<Repo>()?, Arc::clone(&ports.secrets)),
            forge: Arc::clone(&ports.forge),
        });
        Ok(())
    }
}

/// Deletes the source branch of a change, from Igloo's copy and from the forge, once it merged or
/// closed, provided the branch is still at the change's latest revision. The target branch and the repository's default
/// branch are never deleted, and a branch moved since is left as it is.
pub(crate) struct DeleteEndedBranches {
    changes: ChangeQueries,
    repos: RepoQueries,
    forge: Arc<dyn Forge>,
}

#[async_trait]
impl Reactor for DeleteEndedBranches {
    fn name(&self) -> &'static str {
        "platform.delete_ended_branches"
    }

    async fn react(&self, event: &EventEnvelope, _: &CommandBus) -> Result<(), AppError> {
        let Some(id) = event.subject_as::<Change>() else {
            return Ok(());
        };
        match event.decode::<ChangeEvent>()? {
            ChangeEvent::Merged { .. } | ChangeEvent::Closed => {}
            ChangeEvent::Opened { .. }
            | ChangeEvent::Revised { .. }
            | ChangeEvent::Approved { .. }
            | ChangeEvent::Commented { .. }
            | ChangeEvent::ChangesRequested { .. } => return Ok(()),
        }
        let Some(change) = self.changes.get(id).await? else {
            return Ok(());
        };
        let Some(repo) = self.repos.get(change.repo()).await? else {
            return Ok(());
        };
        let source = change.source();
        if source == change.target() || source == repo.default_branch() {
            return Ok(());
        }
        let head = &change.latest().head;
        let remote = self.repos.remote(&repo).await?;
        let deleted = match self.forge.remove(repo.id(), source, head).await {
            Ok(()) => self.forge.delete(&remote, source, head).await,
            Err(error) => Err(error),
        };
        match deleted {
            Ok(()) => Ok(()),
            Err(ForgeError::Moved(_)) => {
                info!(change = %id, branch = %source, "kept a branch that moved after its change ended");
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }
}
