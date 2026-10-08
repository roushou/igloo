use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::ValidationErrors;

/// Key/value metadata on a resource, used to group and filter (`session=42`, `task=fix-flaky`).
///
/// Invariant: at most [`Labels::MAX_ENTRIES`] entries, every key and value valid.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    try_from = "BTreeMap<String, String>",
    into = "BTreeMap<String, String>"
)]
pub struct Labels(BTreeMap<LabelKey, LabelValue>);

/// A label key: 1 to 64 characters of `[a-z0-9._/-]`, starting and ending with `[a-z0-9]`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LabelKey(String);

/// A label value: up to 256 characters, no control characters.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LabelValue(String);

/// Why a label key or value is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LabelError {
    /// The key breaks the key rules.
    #[error(
        "label key must be 1 to 64 characters of [a-z0-9._/-], starting and ending with [a-z0-9]"
    )]
    InvalidKey,
    /// The value is longer than 256 characters.
    #[error("label value must be at most 256 characters")]
    ValueTooLong,
    /// The value contains a control character.
    #[error("label value must not contain control characters")]
    ControlCharacter,
}

impl Labels {
    /// No labels.
    pub const EMPTY: Self = Self(BTreeMap::new());

    /// The maximum number of labels on one resource.
    pub const MAX_ENTRIES: usize = 64;

    /// Labels from key/value pairs, reporting every invalid key, value and the entry limit.
    pub fn from_pairs<I, K, V>(pairs: I) -> Result<Self, ValidationErrors>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        let mut errors = ValidationErrors::default();
        let mut labels = BTreeMap::new();
        for (key, value) in pairs {
            let key = key.as_ref();
            let parsed_key = key.parse::<LabelKey>();
            let parsed_value = LabelValue::try_from(value.into());
            match (parsed_key, parsed_value) {
                (Ok(key), Ok(value)) => {
                    labels.insert(key, value);
                }
                (key_result, value_result) => {
                    if let Err(error) = key_result {
                        errors.add(key, error.to_string());
                    }
                    if let Err(error) = value_result {
                        errors.add(key, error.to_string());
                    }
                }
            }
        }
        if labels.len() > Self::MAX_ENTRIES {
            errors.add("", format!("at most {} labels allowed", Self::MAX_ENTRIES));
        }
        errors.into_result(Self(labels))
    }

    /// The value for `key`, if present.
    #[must_use]
    pub fn get(&self, key: &LabelKey) -> Option<&LabelValue> {
        self.0.get(key)
    }

    /// Entries ordered by key.
    pub fn iter(&self) -> impl Iterator<Item = (&LabelKey, &LabelValue)> {
        self.0.iter()
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no labels.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether every entry of `filter` is present here with an equal value. An empty filter
    /// matches everything.
    #[must_use]
    pub fn matches(&self, filter: &Self) -> bool {
        filter
            .iter()
            .all(|(key, value)| self.0.get(key) == Some(value))
    }
}

impl TryFrom<BTreeMap<String, String>> for Labels {
    type Error = ValidationErrors;

    fn try_from(map: BTreeMap<String, String>) -> Result<Self, Self::Error> {
        Self::from_pairs(map)
    }
}

impl From<Labels> for BTreeMap<String, String> {
    fn from(labels: Labels) -> Self {
        labels
            .0
            .into_iter()
            .map(|(key, value)| (key.0, value.0))
            .collect()
    }
}

impl LabelKey {
    /// The key text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for LabelKey {
    type Err = LabelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();
        let is_edge = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
        let is_inner = |b: &u8| is_edge(b) || matches!(b, b'.' | b'_' | b'/' | b'-');
        let valid = (1..=64).contains(&bytes.len())
            && bytes.first().is_some_and(is_edge)
            && bytes.last().is_some_and(is_edge)
            && bytes.iter().all(is_inner);
        if valid {
            Ok(Self(s.to_owned()))
        } else {
            Err(LabelError::InvalidKey)
        }
    }
}

impl fmt::Display for LabelKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl LabelValue {
    /// The maximum length in characters.
    pub const MAX_CHARS: usize = 256;

    /// The value text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for LabelValue {
    type Error = LabelError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.chars().count() > Self::MAX_CHARS {
            Err(LabelError::ValueTooLong)
        } else if value.chars().any(char::is_control) {
            Err(LabelError::ControlCharacter)
        } else {
            Ok(Self(value))
        }
    }
}

impl fmt::Display for LabelValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_labels() {
        let labels = Labels::from_pairs([("session", "42"), ("igloo.dev/task", "fix-flaky")])
            .expect("valid labels");
        assert_eq!(labels.len(), 2);
    }

    #[test]
    fn reports_every_invalid_entry() {
        let long = "x".repeat(257);
        let errors = Labels::from_pairs([
            ("Upper", "ok"),
            ("-dash", "ok"),
            ("fine", long.as_str()),
            ("tab", "a\tb"),
        ])
        .expect_err("four invalid entries");
        let paths: Vec<&str> = errors.fields().map(|(path, _)| path).collect();
        assert_eq!(paths, ["-dash", "Upper", "fine", "tab"]);
    }

    #[test]
    fn limits_the_number_of_entries() {
        let pairs = (0..=Labels::MAX_ENTRIES).map(|i| (format!("k{i}"), "v"));
        assert!(Labels::from_pairs(pairs).is_err());
    }

    #[test]
    fn filters_by_equality() {
        let labels = Labels::from_pairs([("a", "1"), ("b", "2")]).expect("valid");
        let matching = Labels::from_pairs([("a", "1")]).expect("valid");
        let other = Labels::from_pairs([("a", "2")]).expect("valid");
        assert!(labels.matches(&matching));
        assert!(labels.matches(&Labels::default()));
        assert!(!labels.matches(&other));
    }

    #[test]
    fn serializes_as_a_map() {
        let labels = Labels::from_pairs([("session", "42"), ("task", "fix-flaky")]).expect("valid");
        insta::assert_json_snapshot!(labels);
        let invalid = serde_json::from_str::<Labels>(r#"{"Bad":"x"}"#);
        assert!(invalid.is_err());
    }
}
