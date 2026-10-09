//! The Igloo worker: converges local sandboxes to the assignment received from the server and
//! runs leased jobs in them, through the `SandboxRuntime` abstraction.
//!
//! Talks to the server only through `igloo-worker-protocol`.

mod blobs;
mod config;
mod executor;
mod layers;
mod reconciler;
mod rootfs;
mod runtime;
mod seal;
mod store;
mod usage;
mod worker;

#[cfg(test)]
mod tests;

pub use blobs::{BlobSource, BlobSourceError, HttpBlobSource, RemoteLayer};
pub use config::WorkerConfig;
pub use runtime::{
    ExitOutcome, LocalSandbox, OciRuntime, OutputChunk, Process, ProcessRuntime, RuntimeError,
    SandboxRuntime,
};
pub use worker::{Worker, WorkerError};
