use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use igloo_core::repo::{RepoId, SecretName};

use crate::ports::{SecretStore, SecretValue, StorageError};

/// Secrets in a map, unencrypted.
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    secrets: Mutex<BTreeMap<(RepoId, SecretName), SecretValue>>,
}

impl MemorySecretStore {
    fn secrets(&self) -> std::sync::MutexGuard<'_, BTreeMap<(RepoId, SecretName), SecretValue>> {
        self.secrets.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl SecretStore for MemorySecretStore {
    async fn put(
        &self,
        repo: RepoId,
        name: &SecretName,
        value: &SecretValue,
    ) -> Result<(), StorageError> {
        self.secrets().insert((repo, name.clone()), value.clone());
        Ok(())
    }

    async fn get(
        &self,
        repo: RepoId,
        name: &SecretName,
    ) -> Result<Option<SecretValue>, StorageError> {
        Ok(self.secrets().get(&(repo, name.clone())).cloned())
    }

    async fn names(&self, repo: RepoId) -> Result<Vec<SecretName>, StorageError> {
        Ok(self
            .secrets()
            .keys()
            .filter(|(owner, _)| *owner == repo)
            .map(|(_, name)| name.clone())
            .collect())
    }

    async fn delete(&self, repo: RepoId, name: &SecretName) -> Result<bool, StorageError> {
        Ok(self.secrets().remove(&(repo, name.clone())).is_some())
    }
}
