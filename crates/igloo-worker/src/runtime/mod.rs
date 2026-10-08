//! Sandbox runtimes: where and how sandboxes hold processes.

mod child;
mod netns;
mod oci;
mod process;

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use igloo_core::Generation;
use igloo_core::process::OutputStream;
use igloo_core::sandbox::{NetworkPolicy, ResourceLimits, SandboxId};
use igloo_core::worker::RuntimeKind;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub use oci::OciRuntime;
pub use process::ProcessRuntime;

/// A sandbox present on this worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalSandbox {
    /// Its id.
    pub id: SandboxId,
    /// The spec generation the worker last acted on.
    pub generation: Generation,
    /// Its materialized file system; processes run in its `workspace/` directory.
    pub root: PathBuf,
    /// Environment of every process in it.
    pub env: BTreeMap<String, String>,
    /// CPU and memory bounds, enforced by isolating runtimes.
    pub limits: ResourceLimits,
    /// Network access, enforced by isolating runtimes.
    pub network: NetworkPolicy,
}

/// A process to run in a sandbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    /// The program and its arguments; never empty.
    pub argv: Vec<String>,
    /// Environment layered over the sandbox's.
    pub env: BTreeMap<String, String>,
    /// How long it may run before it is killed.
    pub timeout: Duration,
}

/// Output read from a running process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputChunk {
    /// The stream it was read from.
    pub stream: OutputStream,
    /// Its byte position within that stream.
    pub offset: u64,
    /// The bytes.
    pub data: Vec<u8>,
}

/// How a process ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitOutcome {
    /// It exited with this code (128 + signal when killed by a signal).
    Exited(i32),
    /// It ran past its timeout and was killed.
    TimedOut,
    /// It was cancelled and killed.
    Cancelled,
}

/// Why a runtime operation failed.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// The process could not be started.
    #[error("could not start the process")]
    Spawn(#[source] io::Error),
    /// The runtime failed.
    #[error("runtime failure")]
    Io(#[from] io::Error),
}

/// Holds sandboxes and runs processes in them.
#[async_trait]
pub trait SandboxRuntime: Send + Sync {
    /// Which runtime this is, as advertised to the server.
    fn kind(&self) -> RuntimeKind;

    /// Prepares `sandbox`, whose file system is already materialized.
    async fn start(&self, sandbox: &LocalSandbox) -> Result<(), RuntimeError>;

    /// Runs `process` in `sandbox`, sending its output to `output`, until it exits, times out
    /// or `cancel` fires.
    async fn exec(
        &self,
        sandbox: &LocalSandbox,
        process: Process,
        output: mpsc::Sender<OutputChunk>,
        cancel: CancellationToken,
    ) -> Result<ExitOutcome, RuntimeError>;

    /// Releases what `start` acquired. The caller removes the file system.
    async fn stop(&self, sandbox: &LocalSandbox) -> Result<(), RuntimeError>;
}
