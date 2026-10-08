//! A `key=value` command-line label.

use std::str::FromStr;

/// A `key=value` label argument.
#[derive(Clone)]
pub(crate) struct Label(String, String);

impl FromStr for Label {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.split_once('=')
            .map(|(key, value)| Self(key.to_owned(), value.to_owned()))
            .ok_or_else(|| format!("{s:?} is not key=value"))
    }
}

impl From<Label> for (String, String) {
    fn from(Label(key, value): Label) -> Self {
        (key, value)
    }
}
