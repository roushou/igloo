use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use igloo_core::worker::RuntimeKind;
use igloo_core::{Labels, ValidationErrors, Validator};

use crate::rootfs::Rootfs;

/// Every variable the worker reads. Any other `IGLOO_WORKER_*` variable is an error.
const VARIABLES: [&str; 9] = [
    "IGLOO_WORKER_SERVER",
    "IGLOO_WORKER_JOIN_TOKEN",
    "IGLOO_WORKER_DATA_DIR",
    "IGLOO_WORKER_RUNTIME",
    "IGLOO_WORKER_DEV_MODE",
    "IGLOO_WORKER_OCI_RUNTIME",
    "IGLOO_WORKER_OVERLAY",
    "IGLOO_WORKER_LAYER_CACHE_MIB",
    "IGLOO_WORKER_LABELS",
];
/// The prefix of the worker's variables.
const PREFIX: &str = "IGLOO_WORKER_";

/// The OCI runtime binary when `IGLOO_WORKER_OCI_RUNTIME` is unset.
const DEFAULT_OCI_RUNTIME: &str = "youki";
/// The layer cache budget, in MiB, when `IGLOO_WORKER_LAYER_CACHE_MIB` is unset.
const DEFAULT_LAYER_CACHE_MIB: u64 = 20 * 1024;

/// Worker configuration, read from `IGLOO_WORKER_*` variables. Every `WorkerConfig` is complete
/// and valid. `docs/configuration.md` lists every variable with its default.
#[derive(Clone, Debug)]
pub struct WorkerConfig {
    pub(crate) server: String,
    pub(crate) join_token: String,
    pub(crate) data_dir: PathBuf,
    pub(crate) runtime: RuntimeKind,
    pub(crate) labels: Labels,
    pub(crate) rootfs: Rootfs,
    pub(crate) layer_cache_bytes: u64,
    pub(crate) oci_binary: PathBuf,
}

impl WorkerConfig {
    /// Reads the process environment, as [`WorkerConfig::from_vars`] does.
    pub fn from_env() -> Result<Self, ValidationErrors> {
        Self::from_vars(std::env::vars_os())
    }

    /// Reads name and value pairs such as the process environment. Reports every problem at
    /// once, by variable name: missing and invalid values, and unknown `IGLOO_WORKER_*`
    /// variables. An empty value counts as unset. Labels are written `key=value,other=value`.
    pub fn from_vars<K, V>(vars: impl IntoIterator<Item = (K, V)>) -> Result<Self, ValidationErrors>
    where
        K: Into<OsString>,
        V: Into<OsString>,
    {
        let vars = Vars::new(vars);
        let server = vars.required("IGLOO_WORKER_SERVER").and_then(|server| {
            if server.starts_with("http://") || server.starts_with("https://") {
                Ok(server)
            } else {
                Err("must be an http:// or https:// URL")
            }
        });
        let dev_mode = vars.flag("IGLOO_WORKER_DEV_MODE");
        let runtime = vars
            .required("IGLOO_WORKER_RUNTIME")
            .and_then(|runtime| match runtime.as_str() {
                "process" => Ok(RuntimeKind::Process),
                "oci" | "youki" => Ok(RuntimeKind::Oci),
                "firecracker" => Ok(RuntimeKind::Firecracker),
                _ => Err("must be oci or process"),
            })
            .and_then(|runtime| match (runtime, dev_mode == Ok(true)) {
                (RuntimeKind::Process, true) => Ok(RuntimeKind::Process),
                (RuntimeKind::Process, false) => {
                    Err("the process runtime has no isolation; it requires IGLOO_WORKER_DEV_MODE")
                }
                (RuntimeKind::Oci, _) if cfg!(target_os = "linux") => Ok(RuntimeKind::Oci),
                (RuntimeKind::Oci, _) => Err("the OCI runtime needs Linux"),
                (RuntimeKind::Firecracker, _) => Err("the Firecracker runtime is not available"),
            });
        let oci_binary = vars.text("IGLOO_WORKER_OCI_RUNTIME").and_then(|binary| {
            let binary = PathBuf::from(binary.as_deref().unwrap_or(DEFAULT_OCI_RUNTIME));
            match binary.file_name().and_then(OsStr::to_str) {
                Some("youki" | "crun" | "runc") => Ok(binary),
                _ => Err("must be youki, crun or runc, by name or path"),
            }
        });
        let rootfs = vars.flag("IGLOO_WORKER_OVERLAY").and_then(|overlay| {
            match (overlay, cfg!(target_os = "linux")) {
                (false, _) => Ok(Rootfs::Copy),
                (true, true) => Ok(Rootfs::Overlay),
                (true, false) => Err("overlay file systems need Linux"),
            }
        });
        let layer_cache_mib = vars.text("IGLOO_WORKER_LAYER_CACHE_MIB").and_then(|mib| {
            mib.map_or(Ok(DEFAULT_LAYER_CACHE_MIB), |mib| {
                mib.parse::<u64>()
                    .map_err(|_| "must be a whole number of MiB")
            })
        });
        let labels = match vars.text("IGLOO_WORKER_LABELS") {
            Ok(labels) => Self::labels(labels.as_deref().unwrap_or_default()),
            Err(error) => Err(ValidationErrors::single("", error)),
        };
        let (
            (),
            server,
            join_token,
            data_dir,
            runtime,
            (),
            labels,
            rootfs,
            oci_binary,
            layer_cache_mib,
        ) = Validator::new()
            .nested("", vars.unknown(|name| name.starts_with(PREFIX)))
            .field("IGLOO_WORKER_SERVER", server)
            .field(
                "IGLOO_WORKER_JOIN_TOKEN",
                vars.required("IGLOO_WORKER_JOIN_TOKEN"),
            )
            .field(
                "IGLOO_WORKER_DATA_DIR",
                vars.path("IGLOO_WORKER_DATA_DIR").ok_or("is required"),
            )
            .field("IGLOO_WORKER_RUNTIME", runtime)
            .field("IGLOO_WORKER_DEV_MODE", dev_mode.map(|_| ()))
            .nested("IGLOO_WORKER_LABELS", labels)
            .field("IGLOO_WORKER_OVERLAY", rootfs)
            .field("IGLOO_WORKER_OCI_RUNTIME", oci_binary)
            .field("IGLOO_WORKER_LAYER_CACHE_MIB", layer_cache_mib)
            .finish()?;
        Ok(Self {
            server,
            join_token,
            data_dir,
            runtime,
            labels,
            rootfs,
            oci_binary,
            layer_cache_bytes: layer_cache_mib.saturating_mul(1 << 20),
        })
    }

    /// Parses `key=value,other=value`; an empty text is no labels.
    fn labels(text: &str) -> Result<Labels, ValidationErrors> {
        let mut labels = BTreeMap::new();
        for pair in text.split(',').filter(|pair| !pair.is_empty()) {
            let Some((key, value)) = pair.split_once('=') else {
                return Err(ValidationErrors::single(
                    "",
                    format!("{pair:?} must be written key=value"),
                ));
            };
            labels.insert(key.to_owned(), value.to_owned());
        }
        Labels::try_from(labels)
    }
}

/// Variables by name, read once. Names that are not UTF-8 are dropped; empty values count as
/// unset.
struct Vars(BTreeMap<String, OsString>);

impl Vars {
    fn new<K, V>(vars: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<OsString>,
        V: Into<OsString>,
    {
        Self(
            vars.into_iter()
                .filter_map(|(name, value)| {
                    let value = value.into();
                    let name = name.into().into_string().ok()?;
                    (!value.is_empty()).then_some((name, value))
                })
                .collect(),
        )
    }

    /// The value of `name` as a path.
    fn path(&self, name: &str) -> Option<PathBuf> {
        self.0.get(name).map(PathBuf::from)
    }

    /// The value of `name` as text.
    fn text(&self, name: &str) -> Result<Option<String>, &'static str> {
        self.0
            .get(name)
            .map(|value| value.to_str().map(str::to_owned).ok_or("must be UTF-8"))
            .transpose()
    }

    /// The value of `name`, which must be set.
    fn required(&self, name: &str) -> Result<String, &'static str> {
        self.text(name)?.ok_or("is required")
    }

    /// The value of `name` as `true` or `false`; `false` when unset.
    fn flag(&self, name: &str) -> Result<bool, &'static str> {
        match self.text(name)?.as_deref() {
            None | Some("false") => Ok(false),
            Some("true") => Ok(true),
            Some(_) => Err("must be true or false"),
        }
    }

    /// An error for every variable `owns` claims that is not in [`VARIABLES`].
    fn unknown(&self, owns: impl Fn(&str) -> bool) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::default();
        for name in self.0.keys() {
            if owns(name) && !VARIABLES.contains(&name.as_str()) {
                errors.add(name.as_str(), "is not a known variable");
            }
        }
        errors.into_result(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vec<(&'static str, &'static str)> {
        vec![
            ("IGLOO_WORKER_SERVER", "http://127.0.0.1:7001"),
            ("IGLOO_WORKER_JOIN_TOKEN", "join"),
            ("IGLOO_WORKER_DATA_DIR", "/tmp/igloo-worker"),
            ("IGLOO_WORKER_RUNTIME", "process"),
            ("IGLOO_WORKER_DEV_MODE", "true"),
        ]
    }

    fn fields(errors: &ValidationErrors) -> Vec<&str> {
        errors.fields().map(|(field, _)| field).collect()
    }

    #[test]
    fn a_valid_configuration_is_accepted() {
        let config = WorkerConfig::from_vars(vars()).expect("valid");
        assert_eq!(config.oci_binary, PathBuf::from(DEFAULT_OCI_RUNTIME));
        assert_eq!(config.layer_cache_bytes, DEFAULT_LAYER_CACHE_MIB << 20);
        assert!(matches!(config.rootfs, Rootfs::Copy));
    }

    #[test]
    fn labels_and_the_cache_budget_are_read() {
        let mut vars = vars();
        vars.push(("IGLOO_WORKER_LABELS", "zone=eu,gpu=none"));
        vars.push(("IGLOO_WORKER_LAYER_CACHE_MIB", "1024"));
        let config = WorkerConfig::from_vars(vars).expect("valid");
        assert_eq!(config.layer_cache_bytes, 1024 << 20);
        assert_eq!(
            config.labels,
            Labels::try_from(BTreeMap::from([
                ("zone".to_owned(), "eu".to_owned()),
                ("gpu".to_owned(), "none".to_owned()),
            ]))
            .expect("labels")
        );
    }

    #[test]
    fn the_process_runtime_requires_dev_mode() {
        let vars = vars()
            .into_iter()
            .filter(|(name, _)| *name != "IGLOO_WORKER_DEV_MODE");
        let errors = WorkerConfig::from_vars(vars).expect_err("no isolation without dev mode");
        assert_eq!(fields(&errors), ["IGLOO_WORKER_RUNTIME"]);
    }

    #[test]
    fn every_problem_is_reported() {
        let errors = WorkerConfig::from_vars([
            ("IGLOO_WORKER_SERVER", "grpc://x"),
            ("IGLOO_WORKER_RUNTIME", "vm"),
            ("IGLOO_WORKER_LABELS", "nokey"),
            ("IGLOO_WORKER_LAYER_CACHE_MIB", "lots"),
            ("IGLOO_WORKER_SERVR", "typo"),
        ])
        .expect_err("problems");
        assert_eq!(
            fields(&errors),
            [
                "IGLOO_WORKER_DATA_DIR",
                "IGLOO_WORKER_JOIN_TOKEN",
                "IGLOO_WORKER_LABELS",
                "IGLOO_WORKER_LAYER_CACHE_MIB",
                "IGLOO_WORKER_RUNTIME",
                "IGLOO_WORKER_SERVER",
                "IGLOO_WORKER_SERVR",
            ]
        );
    }

    #[test]
    fn other_variables_are_left_alone() {
        let mut vars = vars();
        vars.push(("IGLOO_LISTEN", "127.0.0.1:7000"));
        vars.push(("PATH", "/usr/bin"));
        assert!(WorkerConfig::from_vars(vars).is_ok());
    }
}
