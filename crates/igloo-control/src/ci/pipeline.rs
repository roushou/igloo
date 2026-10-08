use std::collections::{BTreeMap, BTreeSet};

use igloo_core::process::EnvVars;
use igloo_core::repo::SecretName;
use igloo_core::sandbox::{Isolation, NetworkPolicy, ResourceLimits};
use igloo_core::snapshot::SnapshotId;
use igloo_core::{ValidationErrors, Validator};
use serde::{Deserialize, Serialize};

/// A repository's checks and how to run them, read from `.igloo/pipeline.toml` at the commit
/// being checked. The JSON Schema of the file is `schemas/pipeline.schema.json`.
///
/// Invariant: at least one check; check names are unique.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pipeline {
    /// What the checkout goes over.
    pub base: Base,
    /// How the warm snapshot is built, if the repository has one.
    pub warm: Option<Warm>,
    /// The sandbox checks run in.
    pub sandbox: SandboxSettings,
    /// The checks, in file order.
    pub checks: Vec<CheckSpec>,
}

/// What a repository's checkout goes over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Base {
    /// A container image, imported once per repository.
    Image {
        /// The reference, such as `rust:1.99-slim`.
        image: String,
        /// `os/architecture`; the server's when unset.
        platform: Option<String>,
    },
    /// A snapshot registered on this server.
    Snapshot(SnapshotId),
    /// Nothing: the checkout alone, for runtimes that use the host's tools.
    None,
}

/// How a repository's warm snapshot is built.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warm {
    /// The shell command building the dependencies.
    pub command: String,
    /// The files pinning the dependencies; a change to any of them builds a new warm snapshot.
    pub lockfiles: Vec<String>,
    /// Network access while building; fetching dependencies usually needs it.
    pub network: NetworkPolicy,
    /// How long the build may run, in seconds.
    pub timeout_seconds: u32,
}

/// The sandbox a run's jobs run in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxSettings {
    /// Separation from the host.
    pub isolation: Isolation,
    /// CPU and memory bounds of every sandbox the run starts, the warm build's included.
    #[serde(default)]
    pub limits: ResourceLimits,
    /// Network access for checks.
    pub network: NetworkPolicy,
    /// Environment of every job.
    pub env: EnvVars,
    /// Repository secrets given to every check.
    pub secrets: BTreeSet<SecretName>,
}

/// One check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckSpec {
    /// Its name, unique in the pipeline.
    pub name: String,
    /// The shell command; the check passes when it exits 0.
    pub run: String,
    /// How long it may run, in seconds.
    pub timeout_seconds: u32,
}

/// The file as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPipeline {
    image: Option<String>,
    platform: Option<String>,
    base: Option<String>,
    warm: Option<RawWarm>,
    #[serde(default)]
    sandbox: RawSandbox,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    secrets: Vec<String>,
    #[serde(default)]
    checks: Vec<RawCheck>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWarm {
    command: String,
    #[serde(default)]
    lockfiles: Vec<String>,
    network: Option<NetworkPolicy>,
    timeout_seconds: Option<u32>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSandbox {
    isolation: Option<Isolation>,
    network: Option<NetworkPolicy>,
    millicpus: Option<u32>,
    memory_mib: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCheck {
    name: String,
    run: String,
    timeout_seconds: Option<u32>,
}

impl Pipeline {
    /// Where the file lives in a repository.
    pub const PATH: &'static str = ".igloo/pipeline.toml";
    const DEFAULT_TIMEOUT: u32 = 3600;
    const MAX_TIMEOUT: u32 = 86_400;

    fn timeout(seconds: Option<u32>) -> Result<u32, &'static str> {
        match seconds.unwrap_or(Self::DEFAULT_TIMEOUT) {
            seconds @ 1..=Self::MAX_TIMEOUT => Ok(seconds),
            _ => Err("must be between 1 and 86400 seconds"),
        }
    }
}

impl TryFrom<&str> for Pipeline {
    type Error = ValidationErrors;

    /// Parses and validates the file, reporting every problem.
    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let raw: RawPipeline = toml::from_str(text)
            .map_err(|error| ValidationErrors::single("pipeline", error.message().to_owned()))?;
        let base = match (raw.image, raw.base) {
            (Some(image), None) => Ok(Base::Image {
                image,
                platform: raw.platform,
            }),
            (None, Some(base)) => base
                .parse()
                .map(Base::Snapshot)
                .map_err(|_| "must be a snapshot id".to_owned()),
            (None, None) => Ok(Base::None),
            (Some(_), Some(_)) => Err("set image or base, not both".to_owned()),
        };
        let warm = raw
            .warm
            .map(|warm| {
                Self::timeout(warm.timeout_seconds).map(|timeout_seconds| Warm {
                    command: warm.command,
                    lockfiles: warm.lockfiles,
                    network: warm.network.unwrap_or(NetworkPolicy::AllowAll),
                    timeout_seconds,
                })
            })
            .transpose();
        let mut secrets = BTreeSet::new();
        let mut invalid = ValidationErrors::default();
        for (index, name) in raw.secrets.iter().enumerate() {
            match name.parse::<SecretName>() {
                Ok(name) => {
                    secrets.insert(name);
                }
                Err(error) => invalid.add(index.to_string(), error.to_string()),
            }
        }
        let mut checks = Vec::new();
        let mut names = BTreeSet::new();
        let mut check_errors = ValidationErrors::default();
        if raw.checks.is_empty() {
            check_errors.add("", "at least one check is needed");
        }
        for (index, check) in raw.checks.into_iter().enumerate() {
            if check.name.trim().is_empty() || !names.insert(check.name.clone()) {
                check_errors.add(format!("{index}.name"), "must be present and unique");
            }
            match Self::timeout(check.timeout_seconds) {
                Ok(timeout_seconds) => checks.push(CheckSpec {
                    name: check.name,
                    run: check.run,
                    timeout_seconds,
                }),
                Err(error) => check_errors.add(format!("{index}.timeout_seconds"), error),
            }
        }
        let defaults = ResourceLimits::default();
        let limits = ResourceLimits::new(
            raw.sandbox.millicpus.unwrap_or(defaults.millicpus()),
            raw.sandbox.memory_mib.unwrap_or(defaults.memory_mib()),
        );
        let (base, warm, limits, env, secrets, checks) = Validator::new()
            .field("base", base)
            .field("warm.timeout_seconds", warm)
            .nested("sandbox", limits)
            .nested("env", EnvVars::try_from(raw.env))
            .nested("secrets", invalid.into_result(secrets))
            .nested("checks", check_errors.into_result(checks))
            .finish()?;
        Ok(Self {
            base,
            warm,
            sandbox: SandboxSettings {
                isolation: raw.sandbox.isolation.unwrap_or_default(),
                limits,
                network: raw.sandbox.network.unwrap_or_default(),
                env,
                secrets,
            },
            checks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn igloos_own_pipeline_parses() {
        let text = include_str!("../../../../.igloo/pipeline.toml");
        let pipeline = Pipeline::try_from(text).expect("valid");
        assert!(matches!(pipeline.base, Base::Image { .. }));
        assert!(pipeline.warm.is_some());
        assert!(pipeline.checks.len() >= 3);
    }

    #[test]
    fn the_json_schema_names_every_field() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../../schemas/pipeline.schema.json"))
                .expect("schema");
        let keys = |value: &serde_json::Value| -> Vec<String> {
            let mut keys: Vec<String> = value
                .as_object()
                .expect("properties")
                .keys()
                .cloned()
                .collect();
            keys.sort();
            keys
        };
        let properties = &schema["properties"];
        assert_eq!(
            keys(properties),
            [
                "base", "checks", "env", "image", "platform", "sandbox", "secrets", "warm"
            ]
        );
        assert_eq!(
            keys(&properties["warm"]["properties"]),
            ["command", "lockfiles", "network", "timeout_seconds"]
        );
        assert_eq!(
            keys(&properties["sandbox"]["properties"]),
            ["isolation", "memory_mib", "millicpus", "network"]
        );
        assert_eq!(
            keys(&properties["checks"]["items"]["properties"]),
            ["name", "run", "timeout_seconds"]
        );
    }

    #[test]
    fn every_problem_is_reported() {
        let errors = Pipeline::try_from(
            r#"
            image = "rust:1.99-slim"
            base = "blake3:00"
            secrets = ["lower"]
            [[checks]]
            name = "a"
            run = "true"
            timeout_seconds = 0
            [[checks]]
            name = "a"
            run = "true"
            "#,
        )
        .expect_err("invalid");
        let fields: Vec<&str> = errors.fields().map(|(field, _)| field).collect();
        assert_eq!(
            fields,
            [
                "base",
                "checks.0.timeout_seconds",
                "checks.1.name",
                "secrets.0"
            ]
        );
        let unknown = Pipeline::try_from("colour = \"blue\"\n").expect_err("unknown key");
        assert_eq!(
            unknown.fields().next().map(|(field, _)| field),
            Some("pipeline")
        );
    }

    #[test]
    fn defaults_fill_what_is_left_out() {
        let pipeline = Pipeline::try_from(
            r#"
            [warm]
            command = "cargo fetch"
            [[checks]]
            name = "test"
            run = "cargo test"
            "#,
        )
        .expect("valid");
        assert_eq!(pipeline.base, Base::None);
        let warm = pipeline.warm.expect("warm");
        assert_eq!(warm.network, NetworkPolicy::AllowAll);
        assert_eq!(pipeline.sandbox.network, NetworkPolicy::DenyAll);
        assert_eq!(pipeline.checks[0].timeout_seconds, 3600);
    }
}
