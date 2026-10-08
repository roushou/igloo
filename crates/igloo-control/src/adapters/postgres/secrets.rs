use std::fmt;
use std::str::FromStr;

use async_trait::async_trait;
use chacha20poly1305::aead::{Aead, Generate, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use igloo_core::repo::{RepoId, SecretName};
use sqlx::PgPool;

use super::PgDatabase;
use crate::ports::{SecretStore, SecretValue, StorageError};

/// The key secrets are encrypted with, derived from the server's configured secret.
///
/// Invariant: derived from a secret of at least [`SecretsKey::MIN_SECRET_BYTES`] bytes.
#[derive(Clone)]
pub struct SecretsKey([u8; 32]);

/// A configured secret too short to derive a key from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("must be at least {} bytes", SecretsKey::MIN_SECRET_BYTES)]
pub struct WeakSecretsKey;

/// Secrets in the `repo_secrets` table, encrypted with XChaCha20-Poly1305 and bound to their
/// repository and name.
pub struct PgSecretStore {
    pool: PgPool,
    cipher: XChaCha20Poly1305,
}

impl SecretsKey {
    /// The shortest configured secret accepted.
    pub const MIN_SECRET_BYTES: usize = 32;
    const CONTEXT: &'static str = "igloo repository secrets v1";
}

impl FromStr for SecretsKey {
    type Err = WeakSecretsKey;

    fn from_str(secret: &str) -> Result<Self, Self::Err> {
        if secret.len() < Self::MIN_SECRET_BYTES {
            return Err(WeakSecretsKey);
        }
        Ok(Self(blake3::derive_key(Self::CONTEXT, secret.as_bytes())))
    }
}

impl fmt::Debug for SecretsKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretsKey(..)")
    }
}

impl PgSecretStore {
    /// Secrets over `database`, encrypted with `key`.
    #[must_use]
    pub fn new(database: &PgDatabase, key: &SecretsKey) -> Self {
        Self {
            pool: database.pool().clone(),
            cipher: XChaCha20Poly1305::new(&key.0.into()),
        }
    }

    fn associated(repo: RepoId, name: &SecretName) -> String {
        format!("{repo}\n{name}")
    }
}

#[async_trait]
impl SecretStore for PgSecretStore {
    async fn put(
        &self,
        repo: RepoId,
        name: &SecretName,
        value: &SecretValue,
    ) -> Result<(), StorageError> {
        let nonce = XNonce::generate();
        let aad = Self::associated(repo, name);
        let payload = Payload {
            msg: value.expose().as_bytes(),
            aad: aad.as_bytes(),
        };
        let ciphertext = self
            .cipher
            .encrypt(&nonce, payload)
            .map_err(|_| StorageError::backend(std::io::Error::other("encryption failed")))?;
        sqlx::query!(
            "INSERT INTO repo_secrets (repo_id, name, nonce, ciphertext) VALUES ($1, $2, $3, $4)
             ON CONFLICT (repo_id, name)
             DO UPDATE SET nonce = EXCLUDED.nonce, ciphertext = EXCLUDED.ciphertext",
            repo.to_string(),
            name.as_str(),
            nonce.as_slice(),
            ciphertext,
        )
        .execute(&self.pool)
        .await
        .map_err(StorageError::backend)?;
        Ok(())
    }

    async fn get(
        &self,
        repo: RepoId,
        name: &SecretName,
    ) -> Result<Option<SecretValue>, StorageError> {
        let Some(row) = sqlx::query!(
            "SELECT nonce, ciphertext FROM repo_secrets WHERE repo_id = $1 AND name = $2",
            repo.to_string(),
            name.as_str(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::backend)?
        else {
            return Ok(None);
        };
        let nonce = XNonce::try_from(row.nonce.as_slice())
            .map_err(|_| StorageError::backend(std::io::Error::other("malformed nonce")))?;
        let aad = Self::associated(repo, name);
        let payload = Payload {
            msg: row.ciphertext.as_slice(),
            aad: aad.as_bytes(),
        };
        let plaintext = self
            .cipher
            .decrypt(&nonce, payload)
            .map_err(|_| StorageError::backend(std::io::Error::other("secret does not decrypt")))?;
        let value = String::from_utf8(plaintext).map_err(StorageError::backend)?;
        SecretValue::try_from(value)
            .map(Some)
            .map_err(StorageError::backend)
    }

    async fn names(&self, repo: RepoId) -> Result<Vec<SecretName>, StorageError> {
        let names = sqlx::query_scalar!(
            "SELECT name FROM repo_secrets WHERE repo_id = $1 ORDER BY name",
            repo.to_string(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::backend)?;
        names
            .into_iter()
            .map(|name| name.parse().map_err(StorageError::backend))
            .collect()
    }

    async fn delete(&self, repo: RepoId, name: &SecretName) -> Result<bool, StorageError> {
        let deleted = sqlx::query!(
            "DELETE FROM repo_secrets WHERE repo_id = $1 AND name = $2",
            repo.to_string(),
            name.as_str(),
        )
        .execute(&self.pool)
        .await
        .map_err(StorageError::backend)?;
        Ok(deleted.rows_affected() > 0)
    }
}
