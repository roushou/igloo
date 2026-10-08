use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use igloo_api::problem::Problem;
use igloo_core::{Actor, Digest};
use jiff::SignedDuration;

use super::{ApiError, ApiState};
use crate::app::{AppError, IdempotencyKey};
use crate::ports::{Claim, KeyedRequest, StoredResponse};

/// Makes `POST` requests that carry an `Idempotency-Key` safe to retry: the first one runs and,
/// if it succeeds, its response is kept for [`Idempotency::RETENTION`]; a retry with the same
/// key and request gets that response again, marked `Idempotent-Replayed: true`.
///
/// Invariant: per actor and key, at most one request runs at a time. Reusing a key for another
/// request is refused with `idempotency.key_reused`; a retry while the first request still runs
/// with `idempotency.in_progress`. Failed requests keep no record and can be retried.
pub(crate) struct Idempotency;

impl Idempotency {
    /// How long a completed request is replayed.
    pub(crate) const RETENTION: SignedDuration = SignedDuration::from_hours(24);
    /// How long a request may hold its key without completing; longer means it was lost.
    const CLAIM_TIMEOUT: SignedDuration = SignedDuration::from_hours(1);
    /// The largest request body fingerprinted.
    const MAX_BODY: usize = 1024 * 1024;
    const HEADER: &'static str = "idempotency-key";
    const REPLAYED: &'static str = "idempotent-replayed";

    /// The middleware.
    pub(crate) async fn guard(
        State(state): State<ApiState>,
        request: Request,
        next: Next,
    ) -> Response {
        let key = Self::header(&request, Self::HEADER)
            .filter(|key| IdempotencyKey::try_from(key.as_str()).is_ok());
        let actor = state
            .auth
            .authenticate(Self::header(&request, AUTHORIZATION.as_str()).as_deref());
        match (request.method() == Method::POST, key, actor) {
            (true, Some(key), Some(actor)) => Self::keyed(&state, &actor, &key, request, next)
                .await
                .unwrap_or_else(IntoResponse::into_response),
            // Unkeyed or unauthenticated: the route itself answers.
            _ => next.run(request).await,
        }
    }

    async fn keyed(
        state: &ApiState,
        actor: &Actor,
        key: &str,
        request: Request,
        next: Next,
    ) -> Result<Response, ApiError> {
        let (parts, body) = request.into_parts();
        let body = to_bytes(body, Self::MAX_BODY).await.map_err(|_| {
            ApiError(Box::new(Problem::new(
                413,
                "request.body",
                "Request body too large to make idempotent",
            )))
        })?;
        let now = state.clock.now();
        let claim = KeyedRequest {
            actor,
            key,
            fingerprint: Self::fingerprint(&parts.method, &parts.uri.to_string(), &body),
            now,
            records_before: now.saturating_add(-Self::RETENTION),
            claims_before: now.saturating_add(-Self::CLAIM_TIMEOUT),
        };
        match state
            .idempotency
            .claim(&claim)
            .await
            .map_err(AppError::from)?
        {
            Claim::Claimed => {}
            Claim::Completed(stored) => return Ok(Self::replay(stored)),
            Claim::InProgress => {
                return Err(ApiError(Box::new(Problem::new(
                    409,
                    "idempotency.in_progress",
                    "A request with this idempotency key is still running",
                ))));
            }
            Claim::Mismatch => {
                return Err(ApiError(Box::new(Problem::new(
                    422,
                    "idempotency.key_reused",
                    "The idempotency key was used for another request",
                ))));
            }
        }
        let response = next.run(Request::from_parts(parts, Body::from(body))).await;
        if !response.status().is_success() {
            state
                .idempotency
                .release(actor, key)
                .await
                .map_err(AppError::from)?;
            return Ok(response);
        }
        let (parts, body) = response.into_parts();
        let body = to_bytes(body, usize::MAX)
            .await
            .map_err(AppError::infrastructure)?;
        let stored = StoredResponse {
            status: parts.status.as_u16(),
            body: body.to_vec(),
        };
        state
            .idempotency
            .complete(actor, key, &stored)
            .await
            .map_err(AppError::from)?;
        Ok(Response::from_parts(parts, Body::from(body)))
    }

    fn header(request: &Request, name: &str) -> Option<String> {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    }

    fn fingerprint(method: &Method, uri: &str, body: &Bytes) -> Digest {
        let mut hasher = blake3::Hasher::new();
        hasher.update(method.as_str().as_bytes());
        hasher.update(b"\n");
        hasher.update(uri.as_bytes());
        hasher.update(b"\n");
        hasher.update(body);
        Digest::from_blake3(*hasher.finalize().as_bytes())
    }

    fn replay(stored: StoredResponse) -> Response {
        let status = StatusCode::from_u16(stored.status).unwrap_or(StatusCode::OK);
        let mut response = (status, stored.body).into_response();
        let headers = response.headers_mut();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(Self::REPLAYED, HeaderValue::from_static("true"));
        response
    }
}
