use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use igloo_core::{Entity, Id, Version};

use super::MemoryEventLog;
use crate::ports::{
    CommitMeta, EntityStore, IdGenerator, IdGeneratorExt, NewEvent, StorageError, Versioned,
};

/// Each entity's full history in memory; loading replays it. Commits append to a shared
/// [`MemoryEventLog`].
pub struct MemoryEntityStore<E: Entity> {
    streams: Mutex<HashMap<Id<E>, Vec<E::Event>>>,
    log: Arc<MemoryEventLog>,
    ids: Arc<dyn IdGenerator>,
}

impl<E: Entity> MemoryEntityStore<E> {
    /// A store appending to `log`, naming events with `ids`.
    #[must_use]
    pub fn new(log: Arc<MemoryEventLog>, ids: Arc<dyn IdGenerator>) -> Self {
        Self {
            streams: Mutex::new(HashMap::new()),
            log,
            ids,
        }
    }
}

#[async_trait]
impl<E> EntityStore<E> for MemoryEntityStore<E>
where
    E: Entity + Send + Sync + 'static,
    E::Event: Clone + Sync,
{
    async fn load(&self, id: Id<E>) -> Result<Option<Versioned<E>>, StorageError> {
        let streams = self
            .streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(history) = streams.get(&id) else {
            return Ok(None);
        };
        let entity = E::replay(history.iter().cloned())
            .ok_or_else(|| StorageError::CorruptHistory(id.to_string()))?;
        let version = Version::from(u64::try_from(history.len()).unwrap_or(u64::MAX));
        Ok(Some(Versioned::loaded(entity, version)))
    }

    async fn commit(
        &self,
        entity: &mut Versioned<E>,
        meta: &CommitMeta,
    ) -> Result<(), StorageError> {
        let events = entity.entity_mut().take_events();
        if events.is_empty() {
            return Ok(());
        }
        let encoded = events
            .iter()
            .map(|event| Ok((self.ids.next(), NewEvent::new(event)?)))
            .collect::<Result<Vec<_>, StorageError>>()?;
        let id = entity.entity().id();
        let mut streams = self
            .streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let history = streams.entry(id).or_default();
        let actual = Version::from(u64::try_from(history.len()).unwrap_or(u64::MAX));
        if actual != entity.version() {
            return Err(StorageError::Conflict {
                expected: entity.version(),
                actual,
            });
        }
        self.log
            .append(&id.to_string(), actual.get() + 1, encoded, meta);
        let count = events.len();
        history.extend(events);
        entity.advance(count);
        Ok(())
    }

    async fn ids(&self) -> Result<Vec<Id<E>>, StorageError> {
        let streams = self
            .streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut ids: Vec<Id<E>> = streams.keys().copied().collect();
        ids.sort();
        Ok(ids)
    }
}
