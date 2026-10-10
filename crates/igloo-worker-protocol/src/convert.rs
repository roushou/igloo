use std::collections::BTreeSet;

use igloo_core::job::{FencingToken, Job, JobFailure, JobSpec, Lease};
use igloo_core::process::OutputStream;
use igloo_core::sandbox::{DesiredState, FailureReason, NetworkPolicy, Sandbox, SandboxPhase};
use igloo_core::snapshot::{MediaType, SnapshotLayer};
use igloo_core::terminal::{TerminalFailure, TerminalId, TerminalOutcome, TerminalSize};
use igloo_core::worker::{Arch, Capabilities, NoRuntime, Os, ProtocolVersion, RuntimeKind, Usage};
use igloo_core::{Digest, Entity, Generation, Resource};

use crate::v1;

/// A message that does not describe a valid domain value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    /// A required field is missing or unspecified.
    #[error("{0} is missing or unspecified")]
    Missing(&'static str),
    /// A field holds a value the domain rejects.
    #[error("{field} is invalid: {reason}")]
    Invalid {
        /// The field.
        field: &'static str,
        /// Why.
        reason: String,
    },
}

impl TryFrom<&v1::Hello> for Capabilities {
    type Error = ProtocolError;

    fn try_from(hello: &v1::Hello) -> Result<Self, Self::Error> {
        let capabilities = hello
            .capabilities
            .as_ref()
            .ok_or(ProtocolError::Missing("capabilities"))?;
        let os = match capabilities.os() {
            v1::Os::Linux => Os::Linux,
            v1::Os::Macos => Os::Macos,
            v1::Os::Unspecified => return Err(ProtocolError::Missing("capabilities.os")),
        };
        let arch = match capabilities.arch() {
            v1::Arch::X8664 => Arch::X86_64,
            v1::Arch::Aarch64 => Arch::Aarch64,
            v1::Arch::Unspecified => return Err(ProtocolError::Missing("capabilities.arch")),
        };
        let runtimes = capabilities
            .runtimes()
            .map(|runtime| match runtime {
                v1::Runtime::Process => Ok(RuntimeKind::Process),
                v1::Runtime::Oci => Ok(RuntimeKind::Oci),
                v1::Runtime::Firecracker => Ok(RuntimeKind::Firecracker),
                v1::Runtime::Unspecified => Err(ProtocolError::Missing("capabilities.runtimes")),
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let protocol = ProtocolVersion::from(hello.protocol_version);
        Capabilities::new(os, arch, runtimes, protocol).map_err(|NoRuntime| {
            ProtocolError::Invalid {
                field: "capabilities.runtimes",
                reason: NoRuntime.to_string(),
            }
        })
    }
}

impl From<&Capabilities> for v1::Capabilities {
    fn from(capabilities: &Capabilities) -> Self {
        let mut message = Self::default();
        message.set_os(match capabilities.os() {
            Os::Linux => v1::Os::Linux,
            Os::Macos => v1::Os::Macos,
        });
        message.set_arch(match capabilities.arch() {
            Arch::X86_64 => v1::Arch::X8664,
            Arch::Aarch64 => v1::Arch::Aarch64,
        });
        for runtime in capabilities.runtimes() {
            message.push_runtimes(match runtime {
                RuntimeKind::Process => v1::Runtime::Process,
                RuntimeKind::Oci => v1::Runtime::Oci,
                RuntimeKind::Firecracker => v1::Runtime::Firecracker,
            });
        }
        message
    }
}

impl From<&Usage> for v1::Usage {
    fn from(usage: &Usage) -> Self {
        Self {
            disk_total_bytes: usage.disk_total_bytes(),
            disk_free_bytes: usage.disk_free_bytes(),
            layer_cache_bytes: usage.layer_cache_bytes(),
            layer_cache_limit_bytes: usage.layer_cache_limit_bytes(),
            sandboxes: usage.sandboxes(),
        }
    }
}

impl From<&v1::Usage> for Usage {
    fn from(usage: &v1::Usage) -> Self {
        Self::new(
            usage.disk_total_bytes,
            usage.disk_free_bytes,
            usage.layer_cache_bytes,
            usage.layer_cache_limit_bytes,
            usage.sandboxes,
        )
    }
}

impl From<&Sandbox> for v1::AssignedSandbox {
    fn from(sandbox: &Sandbox) -> Self {
        let spec = sandbox.spec();
        let mut message = Self {
            sandbox_id: sandbox.id().to_string(),
            snapshot: spec.snapshot().to_string(),
            generation: sandbox.generation().get(),
            millicpus: spec.limits().millicpus(),
            memory_mib: spec.limits().memory_mib(),
            env: spec
                .env()
                .iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect(),
            ..Self::default()
        };
        message.set_desired(match spec.desired() {
            DesiredState::Running => v1::DesiredState::Running,
            DesiredState::Stopped => v1::DesiredState::Stopped,
        });
        message.set_network(match spec.network() {
            NetworkPolicy::DenyAll => v1::NetworkPolicy::DenyAll,
            NetworkPolicy::AllowAll => v1::NetworkPolicy::AllowAll,
        });
        message
    }
}

impl TryFrom<v1::NetworkPolicy> for NetworkPolicy {
    type Error = ProtocolError;

    fn try_from(policy: v1::NetworkPolicy) -> Result<Self, Self::Error> {
        match policy {
            v1::NetworkPolicy::DenyAll => Ok(Self::DenyAll),
            v1::NetworkPolicy::AllowAll => Ok(Self::AllowAll),
            v1::NetworkPolicy::Unspecified => Err(ProtocolError::Missing("network")),
        }
    }
}

impl v1::AssignedSandbox {
    /// The assignment with the snapshot's `layers`, lowest first.
    #[must_use]
    pub fn with_layers(mut self, layers: Vec<v1::SnapshotLayer>) -> Self {
        self.layers = layers;
        self
    }
}

impl v1::SnapshotLayer {
    /// `layer`, downloadable from `url`.
    #[must_use]
    pub fn new(layer: &SnapshotLayer, url: impl Into<String>) -> Self {
        let mut message = Self {
            digest: layer.digest().to_string(),
            url: url.into(),
            ..Self::default()
        };
        message.set_media_type(match layer.media_type() {
            MediaType::Tar => v1::LayerMediaType::Tar,
            MediaType::TarGzip => v1::LayerMediaType::TarGzip,
        });
        message
    }
}

impl TryFrom<&v1::SnapshotLayer> for SnapshotLayer {
    type Error = ProtocolError;

    fn try_from(layer: &v1::SnapshotLayer) -> Result<Self, Self::Error> {
        let digest = layer
            .digest
            .parse::<Digest>()
            .map_err(|error| ProtocolError::Invalid {
                field: "digest",
                reason: error.to_string(),
            })?;
        let media_type = match layer.media_type() {
            v1::LayerMediaType::Tar => MediaType::Tar,
            v1::LayerMediaType::TarGzip => MediaType::TarGzip,
            v1::LayerMediaType::Unspecified => return Err(ProtocolError::Missing("media_type")),
        };
        Ok(Self::new(digest, media_type))
    }
}

impl v1::SandboxStatus {
    /// A report that `sandbox` is in `phase` while acting on `observed_generation`.
    #[must_use]
    pub fn new(sandbox: &str, phase: SandboxPhase, observed_generation: Generation) -> Self {
        let mut message = Self {
            sandbox_id: sandbox.to_owned(),
            observed_generation: observed_generation.get(),
            ..Self::default()
        };
        let (phase, failure) = match phase {
            SandboxPhase::Pending | SandboxPhase::Scheduled | SandboxPhase::Starting => {
                (v1::SandboxPhase::Starting, v1::FailureReason::Unspecified)
            }
            SandboxPhase::Running => (v1::SandboxPhase::Running, v1::FailureReason::Unspecified),
            SandboxPhase::Stopping => (v1::SandboxPhase::Stopping, v1::FailureReason::Unspecified),
            SandboxPhase::Stopped => (v1::SandboxPhase::Stopped, v1::FailureReason::Unspecified),
            SandboxPhase::Failed { reason } => (
                v1::SandboxPhase::Failed,
                match reason {
                    FailureReason::SnapshotUnavailable => v1::FailureReason::SnapshotUnavailable,
                    FailureReason::RuntimeError | FailureReason::WorkerLost => {
                        v1::FailureReason::RuntimeError
                    }
                },
            ),
        };
        message.set_phase(phase);
        message.set_failure_reason(failure);
        message
    }
}

impl TryFrom<&v1::SandboxStatus> for SandboxPhase {
    type Error = ProtocolError;

    fn try_from(status: &v1::SandboxStatus) -> Result<Self, Self::Error> {
        Ok(match status.phase() {
            v1::SandboxPhase::Starting => Self::Starting,
            v1::SandboxPhase::Running => Self::Running,
            v1::SandboxPhase::Stopping => Self::Stopping,
            v1::SandboxPhase::Stopped => Self::Stopped,
            v1::SandboxPhase::Failed => Self::Failed {
                reason: match status.failure_reason() {
                    v1::FailureReason::SnapshotUnavailable => FailureReason::SnapshotUnavailable,
                    v1::FailureReason::RuntimeError => FailureReason::RuntimeError,
                    v1::FailureReason::Unspecified => {
                        return Err(ProtocolError::Missing("failure_reason"));
                    }
                },
            },
            v1::SandboxPhase::Unspecified => return Err(ProtocolError::Missing("phase")),
        })
    }
}

impl v1::LeaseGrant {
    /// The grant of `lease` on `job`, renewed by heartbeats within `ttl_seconds`.
    #[must_use]
    pub fn new(job: &Job, lease: &Lease, ttl_seconds: u32) -> Self {
        let JobSpec::Execute {
            sandbox,
            argv,
            env,
            timeout,
            ..
        } = job.spec();
        Self {
            job_id: job.id().to_string(),
            token: lease.token().get(),
            ttl_seconds,
            sandbox_id: sandbox.to_string(),
            argv: argv.clone().into(),
            env: env
                .iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect(),
            timeout_seconds: u32::try_from(timeout.get().as_secs()).unwrap_or(u32::MAX),
        }
    }

    /// The lease's fencing token.
    #[must_use]
    pub fn fencing_token(&self) -> FencingToken {
        FencingToken::from(self.token)
    }
}

impl From<OutputStream> for v1::OutputStream {
    fn from(stream: OutputStream) -> Self {
        match stream {
            OutputStream::Stdout => Self::Stdout,
            OutputStream::Stderr => Self::Stderr,
        }
    }
}

impl TryFrom<v1::OutputStream> for OutputStream {
    type Error = ProtocolError;

    fn try_from(stream: v1::OutputStream) -> Result<Self, Self::Error> {
        match stream {
            v1::OutputStream::Stdout => Ok(Self::Stdout),
            v1::OutputStream::Stderr => Ok(Self::Stderr),
            v1::OutputStream::Unspecified => Err(ProtocolError::Missing("stream")),
        }
    }
}

impl From<JobFailure> for v1::JobFailure {
    fn from(failure: JobFailure) -> Self {
        match failure {
            JobFailure::TimedOut => Self::TimedOut,
            JobFailure::SandboxUnavailable => Self::SandboxUnavailable,
            JobFailure::ExecutionError => Self::ExecutionError,
            JobFailure::LeaseLost => Self::LeaseLost,
        }
    }
}

impl TryFrom<v1::JobFailure> for JobFailure {
    type Error = ProtocolError;

    fn try_from(failure: v1::JobFailure) -> Result<Self, Self::Error> {
        match failure {
            v1::JobFailure::TimedOut => Ok(Self::TimedOut),
            v1::JobFailure::SandboxUnavailable => Ok(Self::SandboxUnavailable),
            v1::JobFailure::ExecutionError => Ok(Self::ExecutionError),
            v1::JobFailure::LeaseLost => Ok(Self::LeaseLost),
            v1::JobFailure::Unspecified => Err(ProtocolError::Missing("failure")),
        }
    }
}

impl From<TerminalSize> for v1::TerminalSize {
    fn from(size: TerminalSize) -> Self {
        Self {
            cols: u32::from(size.cols()),
            rows: u32::from(size.rows()),
        }
    }
}

impl TryFrom<&v1::TerminalSize> for TerminalSize {
    type Error = ProtocolError;

    fn try_from(size: &v1::TerminalSize) -> Result<Self, Self::Error> {
        Self::new(size.cols, size.rows).map_err(|error| ProtocolError::Invalid {
            field: "size",
            reason: error.to_string(),
        })
    }
}

impl From<TerminalFailure> for v1::TerminalFailure {
    fn from(failure: TerminalFailure) -> Self {
        match failure {
            TerminalFailure::SandboxUnavailable => Self::SandboxUnavailable,
            TerminalFailure::ExecutionError => Self::ExecutionError,
            TerminalFailure::Closed => Self::Closed,
        }
    }
}

impl TryFrom<v1::TerminalFailure> for TerminalFailure {
    type Error = ProtocolError;

    fn try_from(failure: v1::TerminalFailure) -> Result<Self, Self::Error> {
        match failure {
            v1::TerminalFailure::SandboxUnavailable => Ok(Self::SandboxUnavailable),
            v1::TerminalFailure::ExecutionError => Ok(Self::ExecutionError),
            v1::TerminalFailure::Closed => Ok(Self::Closed),
            v1::TerminalFailure::Unspecified => Err(ProtocolError::Missing("failure")),
        }
    }
}

impl v1::TerminalExit {
    /// The report that terminal `terminal` ended with `outcome`.
    #[must_use]
    pub fn new(terminal: TerminalId, outcome: TerminalOutcome) -> Self {
        let outcome = match outcome {
            TerminalOutcome::Exited(code) => v1::terminal_exit::Outcome::ExitCode(code),
            TerminalOutcome::Failed(failure) => {
                v1::terminal_exit::Outcome::Failure(v1::TerminalFailure::from(failure).into())
            }
        };
        Self {
            terminal_id: terminal.to_string(),
            outcome: Some(outcome),
        }
    }
}

impl TryFrom<&v1::TerminalExit> for TerminalOutcome {
    type Error = ProtocolError;

    fn try_from(exit: &v1::TerminalExit) -> Result<Self, Self::Error> {
        match exit.outcome {
            Some(v1::terminal_exit::Outcome::ExitCode(code)) => Ok(Self::Exited(code)),
            Some(v1::terminal_exit::Outcome::Failure(failure)) => {
                let failure = v1::TerminalFailure::try_from(failure)
                    .map_err(|_| ProtocolError::Missing("failure"))?;
                TerminalFailure::try_from(failure).map(Self::Failed)
            }
            None => Err(ProtocolError::Missing("outcome")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_round_trip() {
        let capabilities = Capabilities::new(
            Os::Linux,
            Arch::Aarch64,
            BTreeSet::from([RuntimeKind::Process, RuntimeKind::Oci]),
            ProtocolVersion::V1,
        )
        .expect("one runtime");
        let hello = v1::Hello {
            protocol_version: 1,
            capabilities: Some(v1::Capabilities::from(&capabilities)),
            ..v1::Hello::default()
        };
        assert_eq!(Capabilities::try_from(&hello), Ok(capabilities));
    }

    #[test]
    fn usage_round_trips() {
        let usage = Usage::new(100, 40, 7, 20, 3);
        assert_eq!(Usage::from(&v1::Usage::from(&usage)), usage);
    }

    #[test]
    fn a_hello_without_usage_is_still_a_hello() {
        let capabilities = Capabilities::new(
            Os::Linux,
            Arch::X86_64,
            BTreeSet::from([RuntimeKind::Oci]),
            ProtocolVersion::V1,
        )
        .expect("one runtime");
        let hello = v1::Hello {
            protocol_version: 1,
            capabilities: Some(v1::Capabilities::from(&capabilities)),
            ..v1::Hello::default()
        };
        assert!(hello.usage.is_none());
        assert!(Capabilities::try_from(&hello).is_ok());
    }

    #[test]
    fn layers_round_trip() {
        let layer = SnapshotLayer::new(Digest::from_blake3([1; 32]), MediaType::TarGzip);
        let message = v1::SnapshotLayer::new(&layer, "http://blobs.test/one");
        assert_eq!(message.url, "http://blobs.test/one");
        assert_eq!(SnapshotLayer::try_from(&message), Ok(layer));
        let unspecified = v1::SnapshotLayer {
            media_type: 0,
            ..message
        };
        assert_eq!(
            SnapshotLayer::try_from(&unspecified),
            Err(ProtocolError::Missing("media_type"))
        );
    }

    #[test]
    fn unspecified_fields_are_rejected() {
        let hello = v1::Hello {
            protocol_version: 1,
            capabilities: Some(v1::Capabilities::default()),
            ..v1::Hello::default()
        };
        assert_eq!(
            Capabilities::try_from(&hello),
            Err(ProtocolError::Missing("capabilities.os"))
        );
        let status = v1::SandboxStatus::default();
        assert_eq!(
            SandboxPhase::try_from(&status),
            Err(ProtocolError::Missing("phase"))
        );
    }

    #[test]
    fn statuses_round_trip() {
        for phase in [
            SandboxPhase::Starting,
            SandboxPhase::Running,
            SandboxPhase::Stopping,
            SandboxPhase::Stopped,
            SandboxPhase::Failed {
                reason: FailureReason::SnapshotUnavailable,
            },
        ] {
            let status = v1::SandboxStatus::new("sbx_x", phase, Generation::INITIAL);
            assert_eq!(SandboxPhase::try_from(&status), Ok(phase));
        }
    }

    #[test]
    fn terminal_sizes_round_trip_and_reject_empty_screens() {
        let size = TerminalSize::new(132, 43).expect("size");
        assert_eq!(
            TerminalSize::try_from(&v1::TerminalSize::from(size)),
            Ok(size)
        );
        assert!(matches!(
            TerminalSize::try_from(&v1::TerminalSize { cols: 0, rows: 24 }),
            Err(ProtocolError::Invalid { field: "size", .. })
        ));
    }

    #[test]
    fn terminal_exits_round_trip() {
        let terminal: TerminalId = "term_00000000000000000000000007".parse().expect("id");
        for outcome in [
            TerminalOutcome::Exited(0),
            TerminalOutcome::Exited(130),
            TerminalOutcome::Failed(TerminalFailure::SandboxUnavailable),
            TerminalOutcome::Failed(TerminalFailure::ExecutionError),
            TerminalOutcome::Failed(TerminalFailure::Closed),
        ] {
            let exit = v1::TerminalExit::new(terminal, outcome);
            assert_eq!(exit.terminal_id, terminal.to_string());
            assert_eq!(TerminalOutcome::try_from(&exit), Ok(outcome));
        }
        assert_eq!(
            TerminalOutcome::try_from(&v1::TerminalExit::default()),
            Err(ProtocolError::Missing("outcome"))
        );
    }
}
