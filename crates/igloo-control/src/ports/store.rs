use async_trait::async_trait;
use igloo_core::{Entity, Id, Version};

use super::CommitMeta;

/// An entity with the version it was loaded at.
///
/// Invariant: `version` counts the events committed for the entity before the ones it records.
#[derive(Debug)]
pub struct Versioned<E> {
    entity: E,
    version: Version,
}

impl<E> Versioned<E> {
    /// A new entity, never stored.
    #[must_use]
    pub const fn new(entity: E) -> Self {
        Self {
            entity,
            version: Version::INITIAL,
        }
    }

    /// An entity loaded at `version`.
    #[must_use]
    pub const fn loaded(entity: E, version: Version) -> Self {
        Self { entity, version }
    }

    /// The entity.
    #[must_use]
    pub const fn entity(&self) -> &E {
        &self.entity
    }

    /// The entity, for applying changes before a commit.
    pub fn entity_mut(&mut self) -> &mut E {
        &mut self.entity
    }

    /// The version it was loaded or last committed at.
    #[must_use]
    pub const fn version(&self) -> Version {
        self.version
    }

    /// Unwraps the entity.
    #[must_use]
    pub fn into_inner(self) -> E {
        self.entity
    }

    /// Records that `count` more events were committed.
    pub fn advance(&mut self, count: usize) {
        for _ in 0..count {
            self.version = self.version.next();
        }
    }
}

/// Persistence for one entity type: current state plus its events, committed atomically.
#[async_trait]
pub trait EntityStore<E>: Send + Sync
where
    E: Entity + Send + Sync + 'static,
{
    /// The entity with the version it was stored at, if it exists.
    async fn load(&self, id: Id<E>) -> Result<Option<Versioned<E>>, StorageError>;

    /// Commits the events the entity recorded, with its new state, if nobody committed since
    /// it was loaded. Advances the version. Committing no events is a no-op.
    async fn commit(
        &self,
        entity: &mut Versioned<E>,
        meta: &CommitMeta,
    ) -> Result<(), StorageError>;

    /// Every stored id; used for periodic resyncs.
    async fn ids(&self) -> Result<Vec<Id<E>>, StorageError>;

    /// Every stored entity. The default loads each id; an adapter may serve it from a
    /// projection instead.
    async fn all(&self) -> Result<Vec<E>, StorageError> {
        let mut all = Vec::new();
        for id in self.ids().await? {
            if let Some(entity) = self.load(id).await? {
                all.push(entity.into_inner());
            }
        }
        Ok(all)
    }
}

/// Why a storage operation failed.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// Someone committed the entity since it was loaded.
    #[error("version conflict: expected {expected}, found {actual}")]
    Conflict {
        /// The version the change was based on.
        expected: Version,
        /// The stored version.
        actual: Version,
    },
    /// A stored history cannot be rebuilt into an entity.
    #[error("corrupt history for {0}")]
    CorruptHistory(String),
    /// The storage backend failed.
    #[error("storage backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl StorageError {
    /// Wraps a backend error.
    pub fn backend(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Backend(Box::new(error))
    }
}
