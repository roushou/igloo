use std::fmt;
use std::str::FromStr;

use crate::error::GitValueError;

/// A git configuration key: `section.name` or `section.subsection.name`.
///
/// Invariant: the section is ASCII alphanumerics, `-` and `.`; the name starts with an ASCII
/// letter and is ASCII alphanumerics and `-`; a subsection has no newline or NUL.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConfigKey(String);

impl ConfigKey {
    /// The key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ConfigKey {
    type Error = GitValueError;

    fn try_from(key: String) -> Result<Self, Self::Error> {
        let (Some((section, rest)), Some((_, name))) = (key.split_once('.'), key.rsplit_once('.'))
        else {
            return Err(GitValueError::ConfigKey);
        };
        let subsection = rest
            .strip_suffix(name)
            .and_then(|rest| rest.strip_suffix('.'));
        let valid = section.starts_with(|c: char| c.is_ascii_alphanumeric())
            && section
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && name.starts_with(|c: char| c.is_ascii_alphabetic())
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && subsection.is_none_or(|sub| !sub.contains(['\n', '\0']));
        if valid {
            Ok(Self(key))
        } else {
            Err(GitValueError::ConfigKey)
        }
    }
}

impl FromStr for ConfigKey {
    type Err = GitValueError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_owned())
    }
}

impl fmt::Display for ConfigKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_have_a_section_and_a_name() {
        for valid in [
            "core.bare",
            "uploadpack.allowAnySHA1InWant",
            "remote.origin.url",
            "branch.a.b.merge",
        ] {
            assert!(valid.parse::<ConfigKey>().is_ok(), "{valid}");
        }
        for invalid in ["core", ".bare", "core.", "core.1bare", "-c.x", "a.b\nc.d"] {
            assert!(invalid.parse::<ConfigKey>().is_err(), "{invalid:?}");
        }
    }
}
