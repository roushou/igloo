//! RFC 9457 problem details.

use igloo_core::ValidationErrors;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// An error response. `code` is stable and machine-readable; `title` and `detail` are for
/// humans.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct Problem {
    /// A URI identifying the problem type.
    #[serde(rename = "type")]
    pub kind: String,
    /// A short summary of the problem type.
    pub title: String,
    /// The HTTP status code.
    pub status: u16,
    /// What went wrong in this occurrence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The stable error code, such as `"sandbox.not_found"`.
    pub code: String,
    /// Every invalid field, for validation problems.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<FieldError>,
}

/// One invalid field of a request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct FieldError {
    /// The field path, such as `"limits.millicpus"`.
    pub field: String,
    /// Why it is invalid.
    pub message: String,
}

impl Problem {
    /// A problem with `status`, stable `code` and human `title`.
    #[must_use]
    pub fn new(status: u16, code: impl Into<String>, title: impl Into<String>) -> Self {
        let code = code.into();
        Self {
            kind: format!("https://igloo.dev/problems/{code}"),
            title: title.into(),
            status,
            detail: None,
            code,
            errors: Vec::new(),
        }
    }

    /// Adds a human description of this occurrence.
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Adds every field error.
    #[must_use]
    pub fn with_errors(mut self, errors: &ValidationErrors) -> Self {
        self.errors = errors
            .fields()
            .flat_map(|(field, messages)| {
                messages.iter().map(move |message| FieldError {
                    field: field.to_owned(),
                    message: message.clone(),
                })
            })
            .collect();
        self
    }
}
