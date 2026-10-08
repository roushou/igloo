//! Pretty JSON formatting for command output.

use std::fmt;

use serde::Serialize;

/// A command result displayed as pretty JSON, or empty text if serialization fails.
pub(crate) struct Json<T>(T);

impl<T> From<T> for Json<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T: Serialize> fmt::Display for Json<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&serde_json::to_string_pretty(&self.0).unwrap_or_default())
    }
}
