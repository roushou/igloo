use std::fmt;

use async_trait::async_trait;
use igloo_core::{Actor, Timestamp};

use super::AppError;
use crate::ports::{CommitMeta, CorrelationId, EventEnvelope, EventId};

/// A typed request to change state, dispatched through the `CommandBus`.
pub trait Command: Send + 'static {
    /// What a successful command returns.
    type Output: Send + 'static;

    /// The command's name for tracing, policy and MCP: `"<entity>.<verb>"`.
    const NAME: &'static str;

    /// How long the command may take; `None` takes the bus's limit.
    const TIMEOUT: Option<std::time::Duration> = None;
}

/// Executes one command type.
#[async_trait]
pub trait CommandHandler<C: Command>: Send + Sync + 'static {
    /// Runs `command` on behalf of `context`.
    async fn handle(&self, command: C, context: &RequestContext) -> Result<C::Output, AppError>;
}

/// Who sent a command and how it relates to other work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestContext {
    /// Who acts.
    pub actor: Actor,
    /// The request this command belongs to.
    pub correlation_id: CorrelationId,
    /// The event that caused this command, when a reactor or controller sends it.
    pub causation_id: Option<EventId>,
    /// The client's key for safely retrying a create.
    pub idempotency_key: Option<IdempotencyKey>,
}

/// A client-chosen key making a retried request safe.
///
/// Invariant: 1 to 255 visible ASCII characters.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(String);

/// A malformed idempotency key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("idempotency key must be 1 to 255 visible ASCII characters")]
pub struct InvalidIdempotencyKey;

impl RequestContext {
    /// The metadata stamped on events committed at `time` for this request.
    #[must_use]
    pub const fn commit_meta(&self, time: Timestamp) -> CommitMeta {
        CommitMeta {
            actor: self.actor,
            time,
            correlation_id: self.correlation_id,
            causation_id: self.causation_id,
        }
    }

    /// A context for a command `actor` sends in response to `event`: same correlation, with the
    /// event as cause.
    #[must_use]
    pub const fn caused_by(event: &EventEnvelope, actor: Actor) -> Self {
        Self {
            actor,
            correlation_id: event.correlation_id,
            causation_id: Some(event.id),
            idempotency_key: None,
        }
    }

    /// A context for `actor` starting a new request.
    #[must_use]
    pub const fn new(actor: Actor, correlation_id: CorrelationId) -> Self {
        Self {
            actor,
            correlation_id,
            causation_id: None,
            idempotency_key: None,
        }
    }
}

impl TryFrom<&str> for IdempotencyKey {
    type Error = InvalidIdempotencyKey;

    fn try_from(key: &str) -> Result<Self, Self::Error> {
        let valid = (1..=255).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_graphic());
        if valid {
            Ok(Self(key.to_owned()))
        } else {
            Err(InvalidIdempotencyKey)
        }
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
