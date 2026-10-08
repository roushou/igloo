use std::fmt;

use async_trait::async_trait;
use igloo_core::repo::{RepoId, SecretName};

use super::StorageError;

/// A secret's value. `Debug` never shows it.
///
/// Invariant: at most [`SecretValue::MAX_BYTES`] bytes, no NUL.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

/// A value that cannot be a secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a secret is at most 64 KiB of text without NUL")]
pub struct InvalidSecret;

/// Repository secrets, by name.
///
/// Invariant: a secret is only ever readable through the repository it was stored for.
#[async_trait]
pub trait SecretStore: Send + Sync {
    /// Stores `value` as `name` of `repo`, replacing any previous value.
    async fn put(
        &self,
        repo: RepoId,
        name: &SecretName,
        value: &SecretValue,
    ) -> Result<(), StorageError>;

    /// The value of `name` in `repo`, if set.
    async fn get(
        &self,
        repo: RepoId,
        name: &SecretName,
    ) -> Result<Option<SecretValue>, StorageError>;

    /// The names set in `repo`, sorted.
    async fn names(&self, repo: RepoId) -> Result<Vec<SecretName>, StorageError>;

    /// Removes `name` from `repo`; reports whether it was set.
    async fn delete(&self, repo: RepoId, name: &SecretName) -> Result<bool, StorageError>;
}

impl SecretValue {
    /// The largest value.
    pub const MAX_BYTES: usize = 64 * 1024;

    /// The value, for handing to the process that needs it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SecretValue {
    type Error = InvalidSecret;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() <= Self::MAX_BYTES && !value.contains('\0') {
            Ok(Self(value))
        } else {
            Err(InvalidSecret)
        }
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(***)")
    }
}
