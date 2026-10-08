use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

/// An operating system a worker runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Os {
    /// Linux.
    Linux,
    /// macOS.
    Macos,
}

/// A CPU architecture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arch {
    /// 64-bit x86.
    X86_64,
    /// 64-bit ARM.
    Aarch64,
}

/// A sandbox runtime a worker offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    /// Plain processes on the host, without isolation. Development only.
    Process,
    /// Linux containers through an OCI runtime (youki, crun or runc).
    #[serde(alias = "youki")]
    Oci,
    /// Firecracker microVMs.
    Firecracker,
}

/// A worker protocol version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProtocolVersion(u32);

impl ProtocolVersion {
    /// The first protocol version.
    pub const V1: Self = Self(1);
}

impl From<u32> for ProtocolVersion {
    fn from(version: u32) -> Self {
        Self(version)
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// What a worker offers, advertised when it connects.
///
/// Invariant: at least one runtime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawCapabilities", into = "RawCapabilities")]
pub struct Capabilities {
    os: Os,
    arch: Arch,
    runtimes: BTreeSet<RuntimeKind>,
    protocol: ProtocolVersion,
}

#[derive(Serialize, Deserialize)]
struct RawCapabilities {
    os: Os,
    arch: Arch,
    runtimes: BTreeSet<RuntimeKind>,
    protocol: ProtocolVersion,
}

impl TryFrom<RawCapabilities> for Capabilities {
    type Error = NoRuntime;

    fn try_from(raw: RawCapabilities) -> Result<Self, Self::Error> {
        Self::new(raw.os, raw.arch, raw.runtimes, raw.protocol)
    }
}

impl From<Capabilities> for RawCapabilities {
    fn from(capabilities: Capabilities) -> Self {
        Self {
            os: capabilities.os,
            arch: capabilities.arch,
            runtimes: capabilities.runtimes,
            protocol: capabilities.protocol,
        }
    }
}

/// A worker that offers no runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a worker must offer at least one runtime")]
pub struct NoRuntime;

/// What a job or sandbox needs from a worker. Unset fields accept anything.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, bon::Builder)]
pub struct Requirements {
    os: Option<Os>,
    arch: Option<Arch>,
    runtime: Option<RuntimeKind>,
    min_protocol: Option<ProtocolVersion>,
}

/// One requirement a worker does not meet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mismatch {
    /// Wrong operating system.
    Os {
        /// Required.
        required: Os,
        /// Offered.
        offered: Os,
    },
    /// Wrong architecture.
    Arch {
        /// Required.
        required: Arch,
        /// Offered.
        offered: Arch,
    },
    /// The runtime is not offered.
    Runtime {
        /// Required.
        required: RuntimeKind,
    },
    /// The protocol is too old.
    Protocol {
        /// Minimum required.
        required: ProtocolVersion,
        /// Offered.
        offered: ProtocolVersion,
    },
}

impl Capabilities {
    /// Capabilities offering `runtimes`.
    pub fn new(
        os: Os,
        arch: Arch,
        runtimes: BTreeSet<RuntimeKind>,
        protocol: ProtocolVersion,
    ) -> Result<Self, NoRuntime> {
        if runtimes.is_empty() {
            return Err(NoRuntime);
        }
        Ok(Self {
            os,
            arch,
            runtimes,
            protocol,
        })
    }

    /// The operating system.
    #[must_use]
    pub const fn os(&self) -> Os {
        self.os
    }

    /// The architecture.
    #[must_use]
    pub const fn arch(&self) -> Arch {
        self.arch
    }

    /// The offered runtimes.
    #[must_use]
    pub const fn runtimes(&self) -> &BTreeSet<RuntimeKind> {
        &self.runtimes
    }

    /// The protocol version.
    #[must_use]
    pub const fn protocol(&self) -> ProtocolVersion {
        self.protocol
    }

    /// `Ok` when every requirement is met, otherwise every mismatch.
    pub fn satisfies(&self, requirements: &Requirements) -> Result<(), Vec<Mismatch>> {
        let mut mismatches = Vec::new();
        if let Some(required) = requirements.os.filter(|os| *os != self.os) {
            mismatches.push(Mismatch::Os {
                required,
                offered: self.os,
            });
        }
        if let Some(required) = requirements.arch.filter(|arch| *arch != self.arch) {
            mismatches.push(Mismatch::Arch {
                required,
                offered: self.arch,
            });
        }
        if let Some(required) = requirements
            .runtime
            .filter(|runtime| !self.runtimes.contains(runtime))
        {
            mismatches.push(Mismatch::Runtime { required });
        }
        if let Some(required) = requirements
            .min_protocol
            .filter(|minimum| *minimum > self.protocol)
        {
            mismatches.push(Mismatch::Protocol {
                required,
                offered: self.protocol,
            });
        }
        if mismatches.is_empty() {
            Ok(())
        } else {
            Err(mismatches)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux_process() -> Capabilities {
        Capabilities::new(
            Os::Linux,
            Arch::X86_64,
            BTreeSet::from([RuntimeKind::Process]),
            ProtocolVersion::V1,
        )
        .expect("one runtime")
    }

    #[test]
    fn empty_requirements_are_always_met() {
        assert_eq!(linux_process().satisfies(&Requirements::default()), Ok(()));
    }

    #[test]
    fn reports_every_mismatch() {
        let requirements = Requirements::builder()
            .os(Os::Macos)
            .arch(Arch::Aarch64)
            .runtime(RuntimeKind::Oci)
            .min_protocol(ProtocolVersion::from(2))
            .build();
        let mismatches = linux_process()
            .satisfies(&requirements)
            .expect_err("four mismatches");
        assert_eq!(mismatches.len(), 4);
    }

    #[test]
    fn a_worker_needs_a_runtime() {
        let result = Capabilities::new(
            Os::Linux,
            Arch::X86_64,
            BTreeSet::new(),
            ProtocolVersion::V1,
        );
        assert_eq!(result, Err(NoRuntime));
    }
}
