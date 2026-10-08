use std::fmt;

use async_trait::async_trait;
use igloo_core::{Actor, Event, Id, Prefixed, Timestamp};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use super::StorageError;

/// Marker for event ids (`evt_...`).
pub enum EventMarker {}

impl Prefixed for EventMarker {
    const PREFIX: &'static str = "evt";
}

/// Identifies one stored event.
pub type EventId = Id<EventMarker>;

/// Marker for correlation ids (`cor_...`).
pub enum CorrelationMarker {}

impl Prefixed for CorrelationMarker {
    const PREFIX: &'static str = "cor";
}

/// Ties together every event caused, directly or not, by one request.
pub type CorrelationId = Id<CorrelationMarker>;

/// A position in the global event log.
///
/// Invariant: strictly increasing in commit order; `Sequence::START` precedes every event.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Sequence(u64);

impl Sequence {
    /// The position before the first event.
    pub const START: Self = Self(0);

    /// The next position.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// The raw position.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for Sequence {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Display for Sequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Who and what caused a commit; stamped on every event it writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommitMeta {
    /// Who acted.
    pub actor: Actor,
    /// When.
    pub time: Timestamp,
    /// The request the change belongs to.
    pub correlation_id: CorrelationId,
    /// The event that caused the change, when a reactor or controller acted on one.
    pub causation_id: Option<EventId>,
}

/// A stored event with its metadata. Field names follow CloudEvents where they overlap.
///
/// Invariant: `(subject, stream_version)` is unique; `sequence` orders all events globally.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// The event's id.
    pub id: EventId,
    /// The event kind, such as `"igloo.sandbox.created"`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The schema version of `data`.
    pub schema_version: u16,
    /// The id of the entity the event belongs to, such as `"sbx_..."`.
    pub subject: String,
    /// The event's position within its entity's history, starting at 1.
    pub stream_version: u64,
    /// The event's position in the global log.
    pub sequence: Sequence,
    /// When it was committed.
    pub time: Timestamp,
    /// Who caused it.
    pub actor: Actor,
    /// The request it belongs to.
    pub correlation_id: CorrelationId,
    /// The event that caused it, if any.
    pub causation_id: Option<EventId>,
    /// The serialized domain event.
    pub data: serde_json::Value,
}

/// What a store needs to know about one event it is about to append.
#[derive(Debug)]
pub struct NewEvent {
    /// The event kind.
    pub kind: &'static str,
    /// The schema version.
    pub schema_version: u16,
    /// The serialized event.
    pub data: serde_json::Value,
}

impl NewEvent {
    /// Serializes `event`.
    pub fn new<E: Event>(event: &E) -> Result<Self, StorageError> {
        Ok(Self {
            kind: event.kind(),
            schema_version: E::SCHEMA_VERSION,
            data: serde_json::to_value(event).map_err(StorageError::backend)?,
        })
    }
}

impl EventEnvelope {
    /// The entity id this event belongs to, if it is a `T`.
    #[must_use]
    pub fn subject_as<T: Prefixed>(&self) -> Option<Id<T>> {
        self.subject.parse().ok()
    }

    /// The domain event, deserialized.
    pub fn decode<E: Event>(&self) -> Result<E, StorageError> {
        serde_json::from_value(self.data.clone()).map_err(StorageError::backend)
    }
}

/// The global, ordered log of committed events. Stores append to it in the same transaction as
/// the state change; consumers read it in order.
#[async_trait]
pub trait EventLog: Send + Sync {
    /// Up to `limit` events after `after`, in sequence order.
    async fn read(&self, after: Sequence, limit: usize)
    -> Result<Vec<EventEnvelope>, StorageError>;

    /// The latest committed sequence; changes whenever events are appended.
    fn head(&self) -> watch::Receiver<Sequence>;
}
