use axum::Json;
use axum::extract::rejection::{BytesRejection, JsonRejection};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use igloo_api::problem::Problem;
use igloo_core::ErrorCode;

use crate::app::AppError;
use crate::inbound::{BlobUrlRejected, TerminalError};

/// An error response: an RFC 9457 problem with its status.
#[derive(Debug)]
pub struct ApiError(pub(crate) Box<Problem>);

impl ApiError {
    pub(crate) fn unauthenticated() -> Self {
        Self(Box::new(Problem::new(
            401,
            "auth.unauthenticated",
            "Missing or invalid credentials",
        )))
    }

    pub(crate) fn not_found(code: impl Into<String>) -> Self {
        Self(Box::new(Problem::new(404, code, "Not found")))
    }
}

impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        let code = error.code().to_owned();
        let problem = match &error {
            AppError::NotFound { .. } => Problem::new(404, code, "Not found"),
            AppError::Conflict => Problem::new(409, code, "Concurrent change; retry"),
            AppError::Forbidden(denied) => {
                Problem::new(403, code, "Forbidden").with_detail(denied.to_string())
            }
            AppError::Validation(errors) => {
                Problem::new(422, code, "Invalid request").with_errors(errors)
            }
            AppError::Domain { message, .. } => {
                Problem::new(409, code, "Conflicts with the current state").with_detail(message)
            }
            AppError::Timeout { .. } => Problem::new(504, code, "The command timed out"),
            AppError::Unregistered { .. } | AppError::Infrastructure(_) => {
                tracing::error!(%error, cause = ?error, "request failed");
                Problem::new(500, code, "Internal error")
            }
        };
        Self(Box::new(problem))
    }
}

impl From<BlobUrlRejected> for ApiError {
    fn from(rejected: BlobUrlRejected) -> Self {
        let code = match rejected {
            BlobUrlRejected::Expired => "blob.url_expired",
            BlobUrlRejected::Invalid => "blob.url_invalid",
        };
        Self(Box::new(
            Problem::new(403, code, "Forbidden").with_detail(rejected.to_string()),
        ))
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        Self(Box::new(
            Problem::new(400, "request.malformed", "Malformed request body")
                .with_detail(rejection.body_text()),
        ))
    }
}

impl From<BytesRejection> for ApiError {
    fn from(rejection: BytesRejection) -> Self {
        let status = rejection.status();
        Self(Box::new(
            Problem::new(status.as_u16(), "request.body", "Unreadable request body")
                .with_detail(rejection.body_text()),
        ))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut response = (status, Json(*self.0)).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        response
    }
}

impl From<TerminalError> for ApiError {
    fn from(error: TerminalError) -> Self {
        Self(Box::new(
            Problem::new(409, error.code(), "The terminal is unavailable")
                .with_detail(error.to_string()),
        ))
    }
}
