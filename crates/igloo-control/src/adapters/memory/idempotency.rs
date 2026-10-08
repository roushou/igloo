use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use igloo_core::{Actor, Digest, Timestamp};

use crate::ports::{Claim, IdempotencyStore, KeyedRequest, StorageError, StoredResponse};

/// Idempotency records in a map.
#[derive(Debug, Default)]
pub struct MemoryIdempotencyStore {
    records: Mutex<HashMap<(Actor, String), Record>>,
}

#[derive(Debug)]
struct Record {
    fingerprint: Digest,
    claimed_at: Timestamp,
    response: Option<StoredResponse>,
}

impl Record {
    fn is_stale(&self, request: &KeyedRequest<'_>) -> bool {
        match self.response {
            Some(_) => self.claimed_at < request.records_before,
            None => self.claimed_at < request.claims_before,
        }
    }
}

#[async_trait]
impl IdempotencyStore for MemoryIdempotencyStore {
    async fn claim(&self, request: &KeyedRequest<'_>) -> Result<Claim, StorageError> {
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        let slot = (*request.actor, request.key.to_owned());
        if let Some(record) = records.get(&slot)
            && !record.is_stale(request)
        {
            return Ok(if record.fingerprint != request.fingerprint {
                Claim::Mismatch
            } else if let Some(response) = &record.response {
                Claim::Completed(response.clone())
            } else {
                Claim::InProgress
            });
        }
        records.insert(
            slot,
            Record {
                fingerprint: request.fingerprint,
                claimed_at: request.now,
                response: None,
            },
        );
        Ok(Claim::Claimed)
    }

    async fn complete(
        &self,
        actor: &Actor,
        key: &str,
        response: &StoredResponse,
    ) -> Result<(), StorageError> {
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(record) = records.get_mut(&(*actor, key.to_owned())) {
            record.response = Some(response.clone());
        }
        Ok(())
    }

    async fn release(&self, actor: &Actor, key: &str) -> Result<(), StorageError> {
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        let slot = (*actor, key.to_owned());
        if records
            .get(&slot)
            .is_some_and(|record| record.response.is_none())
        {
            records.remove(&slot);
        }
        Ok(())
    }
}
