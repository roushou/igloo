use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::net::SocketAddr;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use igloo_core::{ValidationErrors, Validator};
use reqwest::Url;

use crate::adapters::postgres::SecretsKey;
use crate::inbound::BlobSigningKey;

/// Every variable the server reads. Any other `IGLOO_*` variable is an error, except the
/// worker's and the CLI's.
const VARIABLES: [&str; 13] = [
    "IGLOO_DATABASE_URL",
    "IGLOO_DEV_TOKEN",
    "IGLOO_JOIN_TOKEN",
    "IGLOO_BLOB_KEY",
    "IGLOO_SECRETS_KEY",
    "IGLOO_LISTEN",
    "IGLOO_GATEWAY_LISTEN",
    "IGLOO_DATA_DIR",
    "IGLOO_PUBLIC_URL",
    "IGLOO_LEASE_TTL_SECONDS",
    "IGLOO_COMMAND_TIMEOUT_SECONDS",
    "IGLOO_LOG_FORMAT",
    "IGLOO_EMBEDDED_WORKER",
];
/// The prefix of the worker's variables.
const WORKER_PREFIX: &str = "IGLOO_WORKER_";
/// The CLI's variables.
const CLI_VARIABLES: [&str; 2] = ["IGLOO_API", "IGLOO_TOKEN"];

/// REST listen address when `IGLOO_LISTEN` is unset.
const DEFAULT_LISTEN: &str = "127.0.0.1:7000";
/// Worker gateway listen address when `IGLOO_GATEWAY_LISTEN` is unset.
const DEFAULT_GATEWAY_LISTEN: &str = "127.0.0.1:7001";
/// How long a command may take when `IGLOO_COMMAND_TIMEOUT_SECONDS` is unset.
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a job lease lasts without a heartbeat when `IGLOO_LEASE_TTL_SECONDS` is unset.
const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(30);
/// The user's data directory on macOS, relative to `HOME`.
const MACOS_DATA_HOME: &str = "Library/Application Support";
/// The user's data directory elsewhere, relative to `HOME`, when `XDG_DATA_HOME` is unset.
const XDG_DATA_HOME_FALLBACK: &str = ".local/share";
/// The server's directory inside the user's data directory.
const DATA_DIR_NAME: &str = "igloo";

/// How logs are written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-readable lines.
    #[default]
    Pretty,
    /// One JSON object per line.
    Json,
}

impl FromStr for LogFormat {
    type Err = &'static str;

    /// Parses `pretty` or `json`.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pretty" => Ok(Self::Pretty),
            "json" => Ok(Self::Json),
            _ => Err("must be pretty or json"),
        }
    }
}

/// Server configuration, read from `IGLOO_*` variables. Every `Config` is complete and valid.
/// `docs/configuration.md` lists every variable with its default.
#[derive(Clone, Debug)]
pub struct Config {
    pub(crate) listen: SocketAddr,
    pub(crate) gateway_listen: SocketAddr,
    pub(crate) database_url: String,
    pub(crate) data_dir: PathBuf,
    pub(crate) dev_token: String,
    pub(crate) join_token: String,
    pub(crate) blob_key: BlobSigningKey,
    pub(crate) secrets_key: SecretsKey,
    pub(crate) public_url: Option<Url>,
    pub(crate) command_timeout: Duration,
    pub(crate) lease_ttl: Duration,
    pub(crate) log_format: LogFormat,
    pub(crate) embedded_worker: bool,
}

impl Config {
    /// Reads the process environment, as [`Config::from_vars`] does.
    pub fn from_env() -> Result<Self, ValidationErrors> {
        Self::from_vars(std::env::vars_os())
    }

    /// Reads name and value pairs such as the process environment. Reports every problem at
    /// once, by variable name: missing and invalid values, and unknown `IGLOO_*` variables
    /// other than the worker's (`IGLOO_WORKER_*`) and the CLI's (`IGLOO_API`, `IGLOO_TOKEN`).
    /// An empty value counts as unset. `IGLOO_DATA_DIR` defaults from `HOME` and
    /// `XDG_DATA_HOME`.
    pub fn from_vars<K, V>(vars: impl IntoIterator<Item = (K, V)>) -> Result<Self, ValidationErrors>
    where
        K: Into<OsString>,
        V: Into<OsString>,
    {
        let vars = Vars::new(vars);
        let database_url = vars.required("IGLOO_DATABASE_URL").and_then(|url| {
            if url.starts_with("postgres://") || url.starts_with("postgresql://") {
                Ok(url)
            } else {
                Err("must be a postgres:// URL")
            }
        });
        let blob_key = vars.required("IGLOO_BLOB_KEY").and_then(|secret| {
            secret
                .parse::<BlobSigningKey>()
                .map_err(|_| "must be at least 32 bytes")
        });
        let secrets_key = vars.required("IGLOO_SECRETS_KEY").and_then(|secret| {
            secret
                .parse::<SecretsKey>()
                .map_err(|_| "must be at least 32 bytes")
        });
        let public_url = vars.text("IGLOO_PUBLIC_URL").and_then(|url| {
            url.map(|url| url.parse::<Url>())
                .transpose()
                .map_err(|_| "must be a URL such as https://igloo.example.com")
        });
        let data_dir = vars
            .path("IGLOO_DATA_DIR")
            .or_else(|| Self::user_data_dir(vars.os("HOME"), vars.os("XDG_DATA_HOME")))
            .ok_or("is required when HOME is unset");
        let log_format = vars
            .text("IGLOO_LOG_FORMAT")
            .and_then(|format| format.map_or(Ok(LogFormat::default()), |format| format.parse()));
        let (
            (),
            listen,
            gateway_listen,
            database_url,
            data_dir,
            dev_token,
            join_token,
            blob_key,
            secrets_key,
            public_url,
            command_timeout,
            lease_ttl,
            log_format,
            embedded_worker,
        ) = Validator::new()
            .nested("", vars.unknown(Self::owns))
            .field("IGLOO_LISTEN", vars.address("IGLOO_LISTEN", DEFAULT_LISTEN))
            .field(
                "IGLOO_GATEWAY_LISTEN",
                vars.address("IGLOO_GATEWAY_LISTEN", DEFAULT_GATEWAY_LISTEN),
            )
            .field("IGLOO_DATABASE_URL", database_url)
            .field("IGLOO_DATA_DIR", data_dir)
            .field("IGLOO_DEV_TOKEN", vars.required("IGLOO_DEV_TOKEN"))
            .field("IGLOO_JOIN_TOKEN", vars.required("IGLOO_JOIN_TOKEN"))
            .field("IGLOO_BLOB_KEY", blob_key)
            .field("IGLOO_SECRETS_KEY", secrets_key)
            .field("IGLOO_PUBLIC_URL", public_url)
            .field(
                "IGLOO_COMMAND_TIMEOUT_SECONDS",
                vars.seconds(
                    "IGLOO_COMMAND_TIMEOUT_SECONDS",
                    DEFAULT_COMMAND_TIMEOUT,
                    1..=300,
                ),
            )
            .field(
                "IGLOO_LEASE_TTL_SECONDS",
                vars.seconds("IGLOO_LEASE_TTL_SECONDS", DEFAULT_LEASE_TTL, 1..=3600),
            )
            .field("IGLOO_LOG_FORMAT", log_format)
            .field("IGLOO_EMBEDDED_WORKER", vars.flag("IGLOO_EMBEDDED_WORKER"))
            .finish()?;
        Ok(Self {
            listen,
            gateway_listen,
            database_url,
            data_dir,
            dev_token,
            join_token,
            blob_key,
            secrets_key,
            public_url,
            command_timeout,
            lease_ttl,
            log_format,
            embedded_worker,
        })
    }

    /// How logs are written.
    #[must_use]
    pub const fn log_format(&self) -> LogFormat {
        self.log_format
    }

    /// Where blobs, repository mirrors and the embedded worker's state live.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The REST API's URL as workers reach it, when configured.
    #[must_use]
    pub const fn public_url(&self) -> Option<&Url> {
        self.public_url.as_ref()
    }

    /// Whether a worker runs in the server process.
    #[must_use]
    pub const fn embedded_worker(&self) -> bool {
        self.embedded_worker
    }

    /// Whether `name` is the server's, rather than the worker's or the CLI's.
    fn owns(name: &str) -> bool {
        name.starts_with("IGLOO_")
            && !name.starts_with(WORKER_PREFIX)
            && !CLI_VARIABLES.contains(&name)
    }

    /// `igloo` in the user's data directory, or `None` without a home directory. An empty or
    /// relative `XDG_DATA_HOME` is ignored, as the XDG specification requires.
    fn user_data_dir(home: Option<&OsStr>, xdg_data_home: Option<&OsStr>) -> Option<PathBuf> {
        let home = home.filter(|home| !home.is_empty()).map(PathBuf::from);
        let base = if cfg!(target_os = "macos") {
            home?.join(MACOS_DATA_HOME)
        } else {
            match xdg_data_home.map(PathBuf::from) {
                Some(xdg) if xdg.is_absolute() => xdg,
                _ => home?.join(XDG_DATA_HOME_FALLBACK),
            }
        };
        Some(base.join(DATA_DIR_NAME))
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

    /// The raw value of `name`.
    fn os(&self, name: &str) -> Option<&OsStr> {
        self.0.get(name).map(OsString::as_os_str)
    }

    /// The value of `name` as a path.
    fn path(&self, name: &str) -> Option<PathBuf> {
        self.os(name).map(PathBuf::from)
    }

    /// The value of `name` as text.
    fn text(&self, name: &str) -> Result<Option<String>, &'static str> {
        self.os(name)
            .map(|value| value.to_str().map(str::to_owned).ok_or("must be UTF-8"))
            .transpose()
    }

    /// The value of `name`, which must be set.
    fn required(&self, name: &str) -> Result<String, &'static str> {
        self.text(name)?.ok_or("is required")
    }

    /// The value of `name` as a socket address, or `default`.
    fn address(&self, name: &str, default: &str) -> Result<SocketAddr, &'static str> {
        self.text(name)?
            .as_deref()
            .unwrap_or(default)
            .parse()
            .map_err(|_| "must be an address such as 127.0.0.1:7000")
    }

    /// The value of `name` as whole seconds within `range`, or `default`.
    fn seconds(
        &self,
        name: &str,
        default: Duration,
        range: RangeInclusive<u64>,
    ) -> Result<Duration, String> {
        let Some(value) = self.text(name)? else {
            return Ok(default);
        };
        match value.parse::<u64>() {
            Ok(seconds) if range.contains(&seconds) => Ok(Duration::from_secs(seconds)),
            _ => Err(format!(
                "must be between {} and {}",
                range.start(),
                range.end()
            )),
        }
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

    fn required() -> Vec<(&'static str, &'static str)> {
        vec![
            ("IGLOO_DATABASE_URL", "postgres://localhost/igloo"),
            ("IGLOO_DATA_DIR", "/var/lib/igloo"),
            ("IGLOO_DEV_TOKEN", "dev"),
            ("IGLOO_JOIN_TOKEN", "join"),
            ("IGLOO_BLOB_KEY", "0123456789abcdef0123456789abcdef"),
            ("IGLOO_SECRETS_KEY", "fedcba9876543210fedcba9876543210"),
        ]
    }

    fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        let mut vars = required();
        vars.extend_from_slice(extra);
        vars
    }

    #[test]
    fn defaults_fill_everything_optional() {
        let config = Config::from_vars(required()).expect("valid");
        assert_eq!(config.listen.port(), 7000);
        assert_eq!(config.gateway_listen.port(), 7001);
        assert_eq!(config.command_timeout, DEFAULT_COMMAND_TIMEOUT);
        assert_eq!(config.lease_ttl, DEFAULT_LEASE_TTL);
        assert_eq!(config.public_url, None);
        assert_eq!(config.log_format, LogFormat::Pretty);
        assert!(!config.embedded_worker);
    }

    #[test]
    fn every_variable_is_read() {
        let config = Config::from_vars(with(&[
            ("IGLOO_LISTEN", "0.0.0.0:8000"),
            ("IGLOO_GATEWAY_LISTEN", "0.0.0.0:8001"),
            ("IGLOO_PUBLIC_URL", "https://igloo.example.com"),
            ("IGLOO_LEASE_TTL_SECONDS", "60"),
            ("IGLOO_COMMAND_TIMEOUT_SECONDS", "20"),
            ("IGLOO_LOG_FORMAT", "json"),
            ("IGLOO_EMBEDDED_WORKER", "true"),
        ]))
        .expect("valid");
        assert_eq!(config.listen.port(), 8000);
        assert_eq!(config.gateway_listen.port(), 8001);
        assert_eq!(config.data_dir, PathBuf::from("/var/lib/igloo"));
        assert!(config.public_url.is_some());
        assert_eq!(config.lease_ttl, Duration::from_secs(60));
        assert_eq!(config.command_timeout, Duration::from_secs(20));
        assert_eq!(config.log_format, LogFormat::Json);
        assert!(config.embedded_worker);
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let errors = Config::from_vars([
            ("IGLOO_LISTEN", "nowhere"),
            ("IGLOO_DATABASE_URL", "mysql://x"),
            ("IGLOO_COMMAND_TIMEOUT_SECONDS", "0"),
            ("IGLOO_LEASE_TTL_SECONDS", "soon"),
            ("IGLOO_BLOB_KEY", "short"),
            ("IGLOO_EMBEDDED_WORKER", "yes"),
            ("IGLOO_LISTN", "typo"),
        ])
        .expect_err("invalid");
        let fields: Vec<&str> = errors.fields().map(|(field, _)| field).collect();
        assert_eq!(
            fields,
            [
                "IGLOO_BLOB_KEY",
                "IGLOO_COMMAND_TIMEOUT_SECONDS",
                "IGLOO_DATABASE_URL",
                "IGLOO_DATA_DIR",
                "IGLOO_DEV_TOKEN",
                "IGLOO_EMBEDDED_WORKER",
                "IGLOO_JOIN_TOKEN",
                "IGLOO_LEASE_TTL_SECONDS",
                "IGLOO_LISTEN",
                "IGLOO_LISTN",
                "IGLOO_SECRETS_KEY",
            ]
        );
    }

    #[test]
    fn worker_cli_and_other_variables_are_left_alone() {
        let config = Config::from_vars(with(&[
            ("IGLOO_WORKER_SERVER", "http://127.0.0.1:7001"),
            ("IGLOO_API", "http://127.0.0.1:7000"),
            ("IGLOO_TOKEN", "dev"),
            ("PATH", "/usr/bin"),
        ]));
        assert!(config.is_ok(), "{config:?}");
    }

    #[test]
    fn empty_values_count_as_unset() {
        let config = Config::from_vars(with(&[("IGLOO_LISTEN", "")])).expect("valid");
        assert_eq!(config.listen.port(), 7000);
        let vars = required()
            .into_iter()
            .map(|(name, value)| (name, if name == "IGLOO_DEV_TOKEN" { "" } else { value }));
        let errors = Config::from_vars(vars).expect_err("required");
        assert_eq!(
            errors.fields().map(|(field, _)| field).collect::<Vec<_>>(),
            ["IGLOO_DEV_TOKEN"]
        );
    }

    #[test]
    fn data_dir_defaults_to_the_user_data_directory() {
        let vars: Vec<_> = required()
            .into_iter()
            .filter(|(name, _)| *name != "IGLOO_DATA_DIR")
            .chain([("HOME", "/home/ada")])
            .collect();
        let config = Config::from_vars(vars).expect("valid");
        let expected = if cfg!(target_os = "macos") {
            "/home/ada/Library/Application Support/igloo"
        } else {
            "/home/ada/.local/share/igloo"
        };
        assert_eq!(config.data_dir, PathBuf::from(expected));

        let home = Some(OsStr::new("/home/ada"));
        assert_eq!(
            Config::user_data_dir(home, Some(OsStr::new("relative"))),
            Some(PathBuf::from(expected))
        );
        if !cfg!(target_os = "macos") {
            assert_eq!(
                Config::user_data_dir(home, Some(OsStr::new("/data"))),
                Some(PathBuf::from("/data/igloo"))
            );
        }
        assert_eq!(Config::user_data_dir(None, None), None);
        assert_eq!(Config::user_data_dir(Some(OsStr::new("")), None), None);
    }
}
