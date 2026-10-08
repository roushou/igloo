//! One trait per kind of side effect. Adapters implement them; only composition code names an
//! adapter.

mod blob;
mod checkpoints;
mod clock;
mod event_log;
mod forge;
mod id;
mod idempotency;
mod logs;
mod policy;
mod registry;
mod secrets;
mod store;

#[cfg(test)]
pub(crate) mod conformance;

pub use blob::{BlobError, BlobReader, BlobStore};
pub use checkpoints::Checkpoints;
pub use clock::Clock;
pub use event_log::{
    CommitMeta, CorrelationId, CorrelationMarker, EventEnvelope, EventId, EventLog, EventMarker,
    NewEvent, Sequence,
};
pub use forge::{Expected, Forge, ForgeError, Remote};
pub use id::{IdGenerator, IdGeneratorExt};
pub use idempotency::{Claim, IdempotencyStore, KeyedRequest, StoredResponse};
pub use logs::{LogEntry, LogStore};
pub use policy::{Authorization, Denied, PolicyEngine};
pub use registry::{
    ImageReference, ImageRegistry, InvalidReference, Platform, PulledImage, PulledLayer,
    RegistryError,
};
pub use secrets::{InvalidSecret, SecretStore, SecretValue};
pub use store::{EntityStore, StorageError, Versioned};
