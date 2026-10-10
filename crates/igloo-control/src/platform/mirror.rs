use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use igloo_core::change::{Change, ChangeEvent, ChangePhase};
use igloo_core::repo::{BranchName, Repo, RepoEvent};
use igloo_core::{Entity, ErrorCode as _};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::{ChangeQueries, RepoQueries};
use crate::app::{AppError, CommandBus, Extension, InstallError, PlatformBuilder, Reactor};
use crate::ports::{EventEnvelope, Expected, Forge, ForgeError};

/// Pushes Igloo's copy of repositories to their forges, which only mirror it.
///
/// Invariant: the default branch reaches the forge only as a fast-forward, so a forge that
/// moved on its own is never overwritten; the branch of an open change is pushed as it is,
/// since a revision may rewrite it. A forge that cannot be reached is a failed push, never a
/// failed merge or push to Igloo.
#[derive(Clone)]
pub struct ForgeMirror {
    repos: RepoQueries,
    changes: ChangeQueries,
    forge: Arc<dyn Forge>,
}

impl ForgeMirror {
    /// A mirror over the stores and forge `platform` was given.
    pub fn new(platform: &PlatformBuilder) -> Result<Self, InstallError> {
        let ports = platform.ports();
        Ok(Self {
            repos: RepoQueries::new(platform.store::<Repo>()?, Arc::clone(&ports.secrets)),
            changes: ChangeQueries::new(platform.store::<Change>()?),
            forge: Arc::clone(&ports.forge),
        })
    }

    /// Brings the forge's default branch of `repo` into Igloo's copy, creating the copy when it
    /// does not exist. The copy wins: it moves to the forge's head only when that descends from
    /// it.
    pub async fn host(&self, repo: &Repo) -> Result<(), AppError> {
        let remote = self.repos.remote(repo).await?;
        self.forge.fetch(&remote, repo.default_branch()).await?;
        Ok(())
    }

    /// Pushes `repo`'s default branch to its forge. A copy without the branch, and a forge whose
    /// branch is not an ancestor of the copy's, are left as they are; the latter is logged.
    pub async fn default_branch(&self, repo: &Repo) -> Result<(), AppError> {
        let branch = repo.default_branch();
        match self.push(repo, branch, Expected::FastForward).await {
            Err(ForgeError::Moved(_)) => {
                warn!(repo = %repo.id(), %branch, "the forge's branch is not an ancestor of Igloo's; not mirrored");
                Ok(())
            }
            result => result.map_err(AppError::from),
        }
    }

    /// Pushes the source branch of `change` to its forge while the change is open.
    pub async fn change(&self, change: &Change) -> Result<(), AppError> {
        if *change.phase() != ChangePhase::Open {
            return Ok(());
        }
        let Some(repo) = self.repos.get(change.repo()).await? else {
            return Ok(());
        };
        if change.source() == repo.default_branch() {
            return Ok(());
        }
        self.push(&repo, change.source(), Expected::Any)
            .await
            .map_err(AppError::from)
    }

    /// Brings every repository's default branch from its forge into Igloo's copy, then pushes
    /// the default branches and the open changes' branches to the forges; a failure of one is
    /// logged and does not stop the rest.
    pub async fn sweep(&self) -> Result<(), AppError> {
        for repo in self.repos.all().await? {
            if let Err(error) = self.host(&repo).await {
                warn!(repo = %repo.id(), %error, "fetching from the forge failed");
            }
            if let Err(error) = self.default_branch(&repo).await {
                warn!(repo = %repo.id(), %error, "mirroring the default branch failed");
            }
            for change in self.changes.of_repo(repo.id()).await? {
                if let Err(error) = self.change(&change).await {
                    warn!(change = %change.id(), %error, "mirroring a change's branch failed");
                }
            }
        }
        Ok(())
    }

    /// Sweeps every `interval`, starting one `interval` after the call, until `cancel` fires.
    pub async fn run(self, interval: Duration, cancel: CancellationToken) -> Result<(), AppError> {
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
        loop {
            tokio::select! {
                () = cancel.cancelled() => return Ok(()),
                _ = ticks.tick() => {}
            }
            match self.sweep().await {
                Ok(()) => info!("mirrored repositories to their forges"),
                Err(error) => warn!(%error, "mirroring failed"),
            }
        }
    }

    /// Pushes the copy's `branch` of `repo`; nothing when the copy lacks it.
    async fn push(
        &self,
        repo: &Repo,
        branch: &BranchName,
        expected: Expected,
    ) -> Result<(), ForgeError> {
        let Some(head) = self.forge.mirrored(repo.id(), branch).await? else {
            return Ok(());
        };
        let remote = self
            .repos
            .remote(repo)
            .await
            .map_err(|error| ForgeError::Git(error.to_string()))?;
        self.forge.push(&remote, &head, branch, expected).await
    }
}

/// Pushes to the forge as changes move: a change's branch when it opens or is revised, the
/// default branch when a change merges. Best effort: a failed push is logged and left to the
/// next sweep, so an unreachable forge never holds up other reactions.
pub(crate) struct MirrorChanges {
    mirror: ForgeMirror,
    changes: ChangeQueries,
    repos: RepoQueries,
}

#[async_trait]
impl Reactor for MirrorChanges {
    fn name(&self) -> &'static str {
        "platform.mirror_changes"
    }

    async fn react(&self, event: &EventEnvelope, _: &CommandBus) -> Result<(), AppError> {
        let Some(id) = event.subject_as::<Change>() else {
            return Ok(());
        };
        let merged = match event.decode::<ChangeEvent>()? {
            ChangeEvent::Opened { .. } | ChangeEvent::Revised { .. } => false,
            ChangeEvent::Merged { .. } => true,
            ChangeEvent::Closed
            | ChangeEvent::Approved { .. }
            | ChangeEvent::Commented { .. }
            | ChangeEvent::ChangesRequested { .. } => return Ok(()),
        };
        let Some(change) = self.changes.get(id).await? else {
            return Ok(());
        };
        let result = if merged {
            match self.repos.get(change.repo()).await? {
                Some(repo) => self.mirror.default_branch(&repo).await,
                None => Ok(()),
            }
        } else {
            self.mirror.change(&change).await
        };
        if let Err(error) = result {
            warn!(change = %id, code = error.code(), %error, "mirroring failed; the next sweep retries");
        }
        Ok(())
    }
}

/// Creates Igloo's copy of a repository when it is registered. Best effort: a forge that cannot
/// be reached is retried by the next sweep.
pub(crate) struct HostRepos {
    mirror: ForgeMirror,
    repos: RepoQueries,
}

#[async_trait]
impl Reactor for HostRepos {
    fn name(&self) -> &'static str {
        "platform.host_repos"
    }

    async fn react(&self, event: &EventEnvelope, _: &CommandBus) -> Result<(), AppError> {
        let Some(id) = event.subject_as::<Repo>() else {
            return Ok(());
        };
        if !matches!(event.decode::<RepoEvent>()?, RepoEvent::Registered { .. }) {
            return Ok(());
        }
        let Some(repo) = self.repos.get(id).await? else {
            return Ok(());
        };
        if let Err(error) = self.mirror.host(&repo).await {
            warn!(repo = %id, code = error.code(), %error, "hosting the repository failed; the next sweep retries");
        }
        Ok(())
    }
}

/// Hosts registered repositories and mirrors Igloo's copies to their forges, after changes move
/// and on a schedule.
pub struct MirrorModule;

impl Extension for MirrorModule {
    fn name(&self) -> &'static str {
        "mirror"
    }

    fn install(self: Box<Self>, platform: &mut PlatformBuilder) -> Result<(), InstallError> {
        let secrets = Arc::clone(&platform.ports().secrets);
        platform.reactor(HostRepos {
            mirror: ForgeMirror::new(platform)?,
            repos: RepoQueries::new(platform.store::<Repo>()?, Arc::clone(&secrets)),
        });
        platform.reactor(MirrorChanges {
            mirror: ForgeMirror::new(platform)?,
            changes: ChangeQueries::new(platform.store::<Change>()?),
            repos: RepoQueries::new(platform.store::<Repo>()?, secrets),
        });
        Ok(())
    }
}
