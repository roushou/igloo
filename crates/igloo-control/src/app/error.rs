use igloo_core::{ErrorCode, ValidationErrors};

use crate::ports::{Denied, StorageError};

/// Every way a command can fail, as seen by transports.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// The target entity does not exist.
    #[error("{code}: {id} not found")]
    NotFound {
        /// `"<entity>.not_found"`.
        code: String,
        /// The id looked up.
        id: String,
    },
    /// A concurrent change won; retrying may succeed.
    #[error("conflicting concurrent change")]
    Conflict,
    /// The policy refused the command.
    #[error("forbidden: {0}")]
    Forbidden(#[from] Denied),
    /// The input is invalid.
    #[error("invalid input: {0}")]
    Validation(#[from] ValidationErrors),
    /// The domain rejected the change.
    #[error("{message}")]
    Domain {
        /// The domain error's code.
        code: String,
        /// The domain error's message.
        message: String,
    },
    /// The command did not finish in time.
    #[error("command {command} timed out")]
    Timeout {
        /// The command's name.
        command: &'static str,
    },
    /// No handler is registered for the command.
    #[error("no handler registered for {command}")]
    Unregistered {
        /// The command's name.
        command: &'static str,
    },
    /// A dependency failed. Details stay in the source and logs, never in responses.
    #[error("infrastructure failure")]
    Infrastructure(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl AppError {
    /// A domain rejection, keeping its code and message.
    pub fn domain(error: &(impl std::error::Error + ErrorCode)) -> Self {
        Self::Domain {
            code: error.code().to_owned(),
            message: error.to_string(),
        }
    }

    /// A missing `entity` with `id`.
    pub fn not_found(entity: &str, id: &impl std::fmt::Display) -> Self {
        Self::NotFound {
            code: format!("{entity}.not_found"),
            id: id.to_string(),
        }
    }

    /// A failed dependency.
    pub fn infrastructure(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Infrastructure(Box::new(error))
    }

    /// Whether the command may succeed when retried unchanged: a lost race, a timeout or a
    /// failed dependency. Every other error is a refusal.
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Conflict | Self::Timeout { .. } | Self::Infrastructure(_)
        )
    }
}

impl From<StorageError> for AppError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Conflict { .. } => Self::Conflict,
            other => Self::infrastructure(other),
        }
    }
}

impl ErrorCode for AppError {
    fn code(&self) -> &str {
        match self {
            Self::NotFound { code, .. } | Self::Domain { code, .. } => code,
            Self::Conflict => "command.conflict",
            Self::Forbidden(_) => "auth.forbidden",
            Self::Validation(errors) => errors.code(),
            Self::Timeout { .. } => "command.timeout",
            Self::Unregistered { .. } => "command.unregistered",
            Self::Infrastructure(_) => "internal",
        }
    }
}
