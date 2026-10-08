//! In-memory adapters for tests and local experiments. Every one passes its port's conformance
//! suite.

mod blob;
mod checkpoints;
mod clock;
mod event_log;
mod idempotency;
mod ids;
mod logs;
mod registry;
mod secrets;
mod store;

pub use blob::MemoryBlobStore;
pub use checkpoints::MemoryCheckpoints;
pub use clock::FixedClock;
pub use event_log::MemoryEventLog;
pub use idempotency::MemoryIdempotencyStore;
pub use ids::SequentialIdGenerator;
pub use logs::MemoryLogStore;
pub use registry::MemoryRegistry;
pub use secrets::MemorySecretStore;
pub use store::MemoryEntityStore;
