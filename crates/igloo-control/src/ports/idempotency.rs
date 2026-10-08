use async_trait::async_trait;
use igloo_core::{Actor, Digest, Timestamp};

use super::StorageError;

/// Requests made with an idempotency key and their responses, scoped to the actor that made
/// them.
///
/// Invariant: at most one live record per actor and key. A record is live until it goes stale:
/// a completed one once it was claimed before `records_before`, an uncompleted claim once it
/// was made before `claims_before`.
#[async_trait]
pub trait IdempotencyStore: Send + Sync {
    /// Claims `key` for `actor`'s request identified by `fingerprint`, unless a live record
    /// exists; reports what the caller must do.
    async fn claim(&self, request: &KeyedRequest<'_>) -> Result<Claim, StorageError>;

    /// Stores the response of the request that claimed `key`.
    async fn complete(
        &self,
        actor: &Actor,
        key: &str,
        response: &StoredResponse,
    ) -> Result<(), StorageError>;

    /// Drops the uncompleted claim on `key`, so the request can be retried.
    async fn release(&self, actor: &Actor, key: &str) -> Result<(), StorageError>;
}

/// A request to claim an idempotency key for.
#[derive(Clone, Debug)]
pub struct KeyedRequest<'a> {
    /// Who makes the request.
    pub actor: &'a Actor,
    /// The key it carries.
    pub key: &'a str,
    /// A digest of everything that makes the request what it is (method, path, body).
    pub fingerprint: Digest,
    /// When it is made.
    pub now: Timestamp,
    /// Completed records claimed before this are stale.
    pub records_before: Timestamp,
    /// Uncompleted claims made before this are stale.
    pub claims_before: Timestamp,
}

/// What to do with a keyed request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Claim {
    /// The key is now held for this request: run it, then complete or release the key.
    Claimed,
    /// The same request already completed: replay this response.
    Completed(StoredResponse),
    /// The same request is still running.
    InProgress,
    /// The key was used for another request.
    Mismatch,
}

/// A response kept for replay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredResponse {
    /// The HTTP status.
    pub status: u16,
    /// The body.
    pub body: Vec<u8>,
}
