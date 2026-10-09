//! Workers: the machines that run sandboxes.

use std::collections::BTreeMap;

use igloo_core::worker::{self as domain, Arch, Os, RuntimeKind, Schedulability, Worker};
use igloo_core::{Entity, Resource, Timestamp};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Whether a worker's stream to the server is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WorkerConnection {
    /// The stream is open.
    Connected,
    /// The stream closed; the worker may still come back, see `disconnected_since`.
    Disconnected,
    /// The worker stayed away too long; it registers again under a new id.
    Lost,
}

/// Whether new work may be placed on a worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WorkerSchedulability {
    /// Accepts new work.
    Schedulable,
    /// Finishes current work, accepts nothing new.
    Draining,
}

/// What a worker offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct WorkerCapabilities {
    /// The operating system: `linux` or `macos`.
    pub os: String,
    /// The CPU architecture: `x86_64` or `aarch64`.
    pub arch: String,
    /// The sandbox runtimes it offers: `process`, `oci` or `firecracker`.
    pub runtimes: Vec<String>,
    /// The worker protocol version it speaks.
    pub protocol: String,
}

/// What the sandboxes placed on a worker hold of it, by their limits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct WorkerAllocation {
    /// Sandboxes that are not stopped or failed.
    pub sandboxes: u32,
    /// Their CPU limits, in millicpus, summed.
    pub millicpus: u64,
    /// Their memory limits, in MiB, summed.
    pub memory_mib: u64,
}

/// A worker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct WorkerResource {
    /// Its id (`wrk_...`).
    pub id: String,
    /// Its labels.
    pub labels: BTreeMap<String, String>,
    /// Its connection.
    pub connection: WorkerConnection,
    /// When its stream closed, while disconnected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub disconnected_since: Option<Timestamp>,
    /// Whether new work may be placed on it by spec.
    pub schedulability: WorkerSchedulability,
    /// Whether new work may be placed on it now: connected and schedulable.
    pub schedulable: bool,
    /// What it offers.
    pub capabilities: WorkerCapabilities,
    /// What its sandboxes hold.
    pub allocated: WorkerAllocation,
}

impl WorkerAllocation {
    /// An allocation of `sandboxes` holding `millicpus` and `memory_mib` in total.
    #[must_use]
    pub const fn new(sandboxes: u32, millicpus: u64, memory_mib: u64) -> Self {
        Self {
            sandboxes,
            millicpus,
            memory_mib,
        }
    }
}

impl WorkerResource {
    /// Sets what the worker's sandboxes hold.
    #[must_use]
    pub const fn with_allocation(mut self, allocated: WorkerAllocation) -> Self {
        self.allocated = allocated;
        self
    }
}

impl From<&Worker> for WorkerResource {
    fn from(worker: &Worker) -> Self {
        let (connection, disconnected_since) = match worker.status() {
            domain::Connection::Connected => (WorkerConnection::Connected, None),
            domain::Connection::Disconnected { since } => {
                (WorkerConnection::Disconnected, Some(*since))
            }
            domain::Connection::Lost => (WorkerConnection::Lost, None),
        };
        let capabilities = worker.capabilities();
        Self {
            id: worker.id().to_string(),
            labels: worker
                .labels()
                .iter()
                .map(|(key, value)| (key.as_str().to_owned(), value.as_str().to_owned()))
                .collect(),
            connection,
            disconnected_since,
            schedulability: match worker.spec() {
                Schedulability::Schedulable => WorkerSchedulability::Schedulable,
                Schedulability::Draining => WorkerSchedulability::Draining,
            },
            schedulable: worker.is_schedulable(),
            capabilities: WorkerCapabilities {
                os: match capabilities.os() {
                    Os::Linux => "linux",
                    Os::Macos => "macos",
                }
                .to_owned(),
                arch: match capabilities.arch() {
                    Arch::X86_64 => "x86_64",
                    Arch::Aarch64 => "aarch64",
                }
                .to_owned(),
                runtimes: capabilities
                    .runtimes()
                    .iter()
                    .map(|runtime| {
                        match runtime {
                            RuntimeKind::Process => "process",
                            RuntimeKind::Oci => "oci",
                            RuntimeKind::Firecracker => "firecracker",
                        }
                        .to_owned()
                    })
                    .collect(),
                protocol: capabilities.protocol().to_string(),
            },
            allocated: WorkerAllocation::default(),
        }
    }
}
