//! Value objects describing a process to run: its arguments and environment.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ValidationErrors;

/// One of a process's output streams.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// Environment variables for a process.
///
/// Invariant: at most [`EnvVars::MAX_ENTRIES`] entries; keys match `[A-Z_][A-Z0-9_]*`; values
/// contain no NUL byte.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    try_from = "BTreeMap<String, String>",
    into = "BTreeMap<String, String>"
)]
pub struct EnvVars(BTreeMap<String, String>);

impl EnvVars {
    /// The maximum number of variables.
    pub const MAX_ENTRIES: usize = 128;

    /// Variables from key/value pairs, reporting every invalid key and value.
    pub fn from_pairs<I, K, V>(pairs: I) -> Result<Self, ValidationErrors>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut errors = ValidationErrors::default();
        let mut vars = BTreeMap::new();
        for (key, value) in pairs {
            let (key, value) = (key.into(), value.into());
            let valid_key = key
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_uppercase() || b == b'_')
                && key
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
            if !valid_key {
                errors.add(key.as_str(), "key must match [A-Z_][A-Z0-9_]*");
            }
            if value.contains('\0') {
                errors.add(key.as_str(), "value must not contain NUL");
            }
            vars.insert(key, value);
        }
        if vars.len() > Self::MAX_ENTRIES {
            errors.add(
                "",
                format!("at most {} variables allowed", Self::MAX_ENTRIES),
            );
        }
        errors.into_result(Self(vars))
    }

    /// Variables ordered by key.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    /// These variables with `over`'s set on top; an error past [`Self::MAX_ENTRIES`].
    pub fn overlaid(&self, over: &Self) -> Result<Self, ValidationErrors> {
        let mut vars = self.0.clone();
        vars.extend(
            over.0
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        Self::from_pairs(vars)
    }

    /// Number of variables.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no variables.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl TryFrom<BTreeMap<String, String>> for EnvVars {
    type Error = ValidationErrors;

    fn try_from(map: BTreeMap<String, String>) -> Result<Self, Self::Error> {
        Self::from_pairs(map)
    }
}

impl From<EnvVars> for BTreeMap<String, String> {
    fn from(vars: EnvVars) -> Self {
        vars.0
    }
}

/// The program and arguments of a process.
///
/// Invariant: at least one element (the program), at most [`Argv::MAX_ARGS`], none containing
/// a NUL byte.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<String>", into = "Vec<String>")]
pub struct Argv(Vec<String>);

/// Why an argument vector is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArgvError {
    /// No program was given.
    #[error("argv must name a program")]
    Empty,
    /// More than [`Argv::MAX_ARGS`] elements.
    #[error("argv must have at most {max} elements", max = Argv::MAX_ARGS)]
    TooLong,
    /// An element contains a NUL byte.
    #[error("argv element {0} contains NUL")]
    Nul(usize),
}

impl Argv {
    /// The maximum number of elements, program included.
    pub const MAX_ARGS: usize = 4096;

    /// The program to run.
    #[must_use]
    pub fn program(&self) -> &str {
        self.0.first().map_or("", String::as_str)
    }

    /// The arguments after the program.
    #[must_use]
    pub fn args(&self) -> &[String] {
        self.0.get(1..).unwrap_or_default()
    }
}

impl TryFrom<Vec<String>> for Argv {
    type Error = ArgvError;

    fn try_from(argv: Vec<String>) -> Result<Self, Self::Error> {
        if argv.is_empty() {
            return Err(ArgvError::Empty);
        }
        if argv.len() > Self::MAX_ARGS {
            return Err(ArgvError::TooLong);
        }
        if let Some(index) = argv.iter().position(|arg| arg.contains('\0')) {
            return Err(ArgvError::Nul(index));
        }
        Ok(Self(argv))
    }
}

impl From<Argv> for Vec<String> {
    fn from(argv: Argv) -> Self {
        argv.0
    }
}

impl fmt::Display for Argv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_vars_report_every_invalid_entry() {
        let errors = EnvVars::from_pairs([("PATH", "/bin"), ("lower", "x"), ("OK", "a\0b")])
            .expect_err("two invalid entries");
        let paths: Vec<&str> = errors.fields().map(|(path, _)| path).collect();
        assert_eq!(paths, ["OK", "lower"]);
    }

    #[test]
    fn argv_needs_a_program() {
        assert_eq!(Argv::try_from(Vec::new()), Err(ArgvError::Empty));
        let argv = Argv::try_from(vec!["cargo".to_owned(), "test".to_owned()]).expect("valid");
        assert_eq!(argv.program(), "cargo");
        assert_eq!(argv.args(), ["test"]);
    }
}
