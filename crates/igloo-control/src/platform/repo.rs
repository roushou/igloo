use std::sync::Arc;

use async_trait::async_trait;
use igloo_core::repo::{BranchName, Repo, RepoId, RepoLocation, SecretName, WarmSnapshot};
use igloo_core::snapshot::SnapshotId;
use igloo_core::{Digest, Entity, ErrorCode, Timestamp};

use crate::app::{
    AppError, Command, CommandHandler, EntityHandler, Extension, InstallError, PlatformBuilder,
    RequestContext,
};
use crate::ports::{
    Clock, EntityStore, IdGenerator, IdGeneratorExt, Remote, SecretStore, SecretValue,
    StorageError, Versioned,
};

/// Registers a repository.
pub struct RegisterRepo {
    /// Where it is hosted.
    pub location: RepoLocation,
    /// The branch changes merge into.
    pub default_branch: BranchName,
    /// The secret holding the forge token, if the forge needs one.
    pub token: Option<SecretName>,
}

/// Sets or replaces a repository secret.
pub struct SetSecret {
    /// The repository.
    pub repo: RepoId,
    /// The secret.
    pub name: SecretName,
    /// Its value.
    pub value: SecretValue,
}

/// Deletes a repository secret.
pub struct DeleteSecret {
    /// The repository.
    pub repo: RepoId,
    /// The secret.
    pub name: SecretName,
}

/// Records the warm snapshot built for `key`.
pub struct RecordWarmSnapshot {
    /// The repository.
    pub repo: RepoId,
    /// What the snapshot was built from.
    pub key: Digest,
    /// The snapshot and its commit.
    pub warm: WarmSnapshot,
}

/// Records that the snapshots under `keys` were used. Keys without a snapshot, and keys used
/// within the last hour, record nothing.
pub struct RecordWarmUse {
    /// The repository.
    pub repo: RepoId,
    /// The keys.
    pub keys: Vec<Digest>,
}

/// Forgets the snapshots under `keys` that were last used at or before `unused_since`.
pub struct ForgetWarmSnapshots {
    /// The repository.
    pub repo: RepoId,
    /// The keys.
    pub keys: Vec<Digest>,
    /// The cutoff: a key used after it is kept.
    pub unused_since: Timestamp,
}

impl Command for RecordWarmUse {
    type Output = ();
    const NAME: &'static str = "repo.record_warm_use";
}

impl Command for ForgetWarmSnapshots {
    type Output = ();
    const NAME: &'static str = "repo.forget_warm_snapshots";
}

/// Records the snapshot a container image was imported as.
pub struct RecordImage {
    /// The repository.
    pub repo: RepoId,
    /// The image reference and platform.
    pub image: String,
    /// The snapshot.
    pub snapshot: SnapshotId,
}

impl Command for RecordImage {
    type Output = ();
    const NAME: &'static str = "repo.record_image";
}

impl Command for RecordWarmSnapshot {
    type Output = ();
    const NAME: &'static str = "repo.record_warm_snapshot";
}

impl Command for RegisterRepo {
    type Output = RepoId;
    const NAME: &'static str = "repo.register";
}

impl Command for SetSecret {
    type Output = ();
    const NAME: &'static str = "repo.set_secret";
}

impl Command for DeleteSecret {
    type Output = ();
    const NAME: &'static str = "repo.delete_secret";
}

/// Why a repository command is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RepoError {
    /// Another repository has the same location.
    #[error("a repository with this location is already registered")]
    AlreadyRegistered,
    /// The secret is not set.
    #[error("no such secret")]
    UnknownSecret,
}

impl ErrorCode for RepoError {
    fn code(&self) -> &'static str {
        match self {
            Self::AlreadyRegistered => "repo.already_registered",
            Self::UnknownSecret => "secret.not_found",
        }
    }
}

/// Read access to repositories and what reaching them needs.
#[derive(Clone)]
pub struct RepoQueries {
    store: Arc<dyn EntityStore<Repo>>,
    secrets: Arc<dyn SecretStore>,
}

impl RepoQueries {
    /// Queries over `store`, reading forge tokens from `secrets`.
    #[must_use]
    pub fn new(store: Arc<dyn EntityStore<Repo>>, secrets: Arc<dyn SecretStore>) -> Self {
        Self { store, secrets }
    }

    /// Repository `id`, if registered.
    pub async fn get(&self, id: RepoId) -> Result<Option<Repo>, StorageError> {
        Ok(self.store.load(id).await?.map(|repo| repo.entity().clone()))
    }

    /// Every repository, oldest first.
    pub async fn all(&self) -> Result<Vec<Repo>, StorageError> {
        let mut repos = self.store.all().await?;
        repos.sort_by_key(Entity::id);
        Ok(repos)
    }

    /// The repository at `location`, if registered.
    pub async fn at(&self, location: &RepoLocation) -> Result<Option<Repo>, StorageError> {
        Ok(self
            .store
            .all()
            .await?
            .into_iter()
            .find(|repo| repo.location() == location))
    }

    /// How to reach `repo` on its forge, with its token when it has one.
    pub async fn remote(&self, repo: &Repo) -> Result<Remote, AppError> {
        let token = match repo.token() {
            Some(name) => Some(
                self.secrets
                    .get(repo.id(), name)
                    .await?
                    .ok_or_else(|| AppError::domain(&RepoError::UnknownSecret))?,
            ),
            None => None,
        };
        Ok(Remote {
            repo: repo.id(),
            location: repo.location().clone(),
            token,
        })
    }
}

/// Registers a repository unless its location is taken.
struct RegisterRepoHandler {
    queries: RepoQueries,
    store: Arc<dyn EntityStore<Repo>>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
}

#[async_trait]
impl CommandHandler<RegisterRepo> for RegisterRepoHandler {
    async fn handle(
        &self,
        command: RegisterRepo,
        context: &RequestContext,
    ) -> Result<RepoId, AppError> {
        if self.queries.at(&command.location).await?.is_some() {
            return Err(AppError::domain(&RepoError::AlreadyRegistered));
        }
        let id = self.ids.next::<Repo>();
        let now = self.clock.now();
        let repo = Repo::new(
            id,
            command.location,
            command.default_branch,
            command.token,
            now,
        );
        self.store
            .commit(&mut Versioned::new(repo), &context.commit_meta(now))
            .await?;
        Ok(id)
    }
}

/// Stores a secret, then records its name on the repository.
struct SetSecretHandler {
    store: Arc<dyn EntityStore<Repo>>,
    secrets: Arc<dyn SecretStore>,
    clock: Arc<dyn Clock>,
}

#[async_trait]
impl CommandHandler<SetSecret> for SetSecretHandler {
    async fn handle(&self, command: SetSecret, context: &RequestContext) -> Result<(), AppError> {
        let mut repo = self
            .store
            .load(command.repo)
            .await?
            .ok_or_else(|| AppError::not_found(Repo::NAME, &command.repo))?;
        self.secrets
            .put(command.repo, &command.name, &command.value)
            .await?;
        repo.entity_mut().secret_set(command.name);
        self.store
            .commit(&mut repo, &context.commit_meta(self.clock.now()))
            .await?;
        Ok(())
    }
}

/// Deletes a secret, then records it on the repository.
struct DeleteSecretHandler {
    store: Arc<dyn EntityStore<Repo>>,
    secrets: Arc<dyn SecretStore>,
    clock: Arc<dyn Clock>,
}

#[async_trait]
impl CommandHandler<DeleteSecret> for DeleteSecretHandler {
    async fn handle(
        &self,
        command: DeleteSecret,
        context: &RequestContext,
    ) -> Result<(), AppError> {
        let mut repo = self
            .store
            .load(command.repo)
            .await?
            .ok_or_else(|| AppError::not_found(Repo::NAME, &command.repo))?;
        if !self.secrets.delete(command.repo, &command.name).await? {
            return Err(AppError::not_found("secret", &command.name));
        }
        repo.entity_mut().secret_deleted(command.name);
        self.store
            .commit(&mut repo, &context.commit_meta(self.clock.now()))
            .await?;
        Ok(())
    }
}

/// Repositories and their secrets.
pub struct RepoModule;

impl Extension for RepoModule {
    fn name(&self) -> &'static str {
        "repo"
    }

    fn install(self: Box<Self>, platform: &mut PlatformBuilder) -> Result<(), InstallError> {
        let store = platform.store::<Repo>()?;
        let ports = platform.ports().clone();
        platform.command(RegisterRepoHandler {
            queries: RepoQueries::new(Arc::clone(&store), Arc::clone(&ports.secrets)),
            store: Arc::clone(&store),
            clock: Arc::clone(&ports.clock),
            ids: Arc::clone(&ports.ids),
        })?;
        platform.command(SetSecretHandler {
            store: Arc::clone(&store),
            secrets: Arc::clone(&ports.secrets),
            clock: Arc::clone(&ports.clock),
        })?;
        platform.command(DeleteSecretHandler {
            store: Arc::clone(&store),
            secrets: Arc::clone(&ports.secrets),
            clock: Arc::clone(&ports.clock),
        })?;
        platform.command(EntityHandler::infallible(
            Arc::clone(&store),
            Arc::clone(&ports.clock),
            |command: &RecordImage| command.repo,
            |repo: &mut Repo, command: &RecordImage, _| {
                repo.image_imported(command.image.clone(), command.snapshot);
            },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(&store),
            Arc::clone(&ports.clock),
            |command: &RecordWarmUse| command.repo,
            |repo: &mut Repo, command: &RecordWarmUse, now| {
                for key in &command.keys {
                    repo.warm_used(*key, now);
                }
            },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(&store),
            Arc::clone(&ports.clock),
            |command: &ForgetWarmSnapshots| command.repo,
            |repo: &mut Repo, command: &ForgetWarmSnapshots, _| {
                for key in &command.keys {
                    repo.forget_warm_unused_since(*key, command.unused_since);
                }
            },
        ))?;
        platform.command(EntityHandler::infallible(
            store,
            Arc::clone(&ports.clock),
            |command: &RecordWarmSnapshot| command.repo,
            |repo: &mut Repo, command: &RecordWarmSnapshot, _| {
                repo.record_warm(command.key, command.warm.clone());
            },
        ))
    }
}
