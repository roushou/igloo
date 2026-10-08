use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::extract::Query;
use axum::http::Method;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use igloo_core::{Actor, Digest, SystemComponent, ValidationErrors};

use super::{ApiError, ApiState};
use crate::app::{AppError, IdempotencyKey, RequestContext};
use crate::inbound::{BlobAccess, BlobSignature, BlobUrlRejected};
use crate::ports::{CorrelationId, IdGeneratorExt};

/// Development authentication: one bearer token standing for one human.
#[derive(Clone, Debug)]
pub struct DevToken {
    token: Arc<str>,
    actor: Actor,
}

impl DevToken {
    /// Requests bearing `token` act as `actor`.
    #[must_use]
    pub fn new(token: impl Into<Arc<str>>, actor: Actor) -> Self {
        Self {
            token: token.into(),
            actor,
        }
    }

    pub(in crate::inbound) fn authenticate(&self, header: Option<&str>) -> Option<Actor> {
        let presented = header?.strip_prefix("Bearer ")?;
        (presented == &*self.token).then_some(self.actor)
    }
}

/// The authenticated caller of a request, as a command context.
pub(crate) struct Caller(pub(crate) RequestContext);

impl Caller {
    const CORRELATION: &'static str = "x-correlation-id";
    const IDEMPOTENCY: &'static str = "idempotency-key";
}

impl FromRequestParts<ApiState> for Caller {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &ApiState,
    ) -> impl Future<Output = Result<Self, ApiError>> + Send {
        std::future::ready(Self::extract(parts, state))
    }
}

impl Caller {
    fn extract(parts: &Parts, state: &ApiState) -> Result<Self, ApiError> {
        let header = |name: &str| {
            parts
                .headers
                .get(name)
                .and_then(|value| value.to_str().ok())
        };
        let actor = state
            .auth
            .authenticate(header(AUTHORIZATION.as_str()))
            .ok_or_else(ApiError::unauthenticated)?;
        let correlation_id = header(Self::CORRELATION)
            .and_then(|value| value.parse::<CorrelationId>().ok())
            .unwrap_or_else(|| state.ids.next());
        let idempotency_key = header(Self::IDEMPOTENCY)
            .map(IdempotencyKey::try_from)
            .transpose()
            .map_err(|error| {
                AppError::Validation(ValidationErrors::single(
                    Self::IDEMPOTENCY,
                    error.to_string(),
                ))
            })?;
        let mut context = RequestContext::new(actor, correlation_id);
        context.idempotency_key = idempotency_key;
        Ok(Self(context))
    }
}

/// The caller of a blob route: a bearer token, or a presigned URL for the request's method and
/// digest, which acts as the gateway.
pub(crate) struct BlobCaller(pub(crate) RequestContext);

impl FromRequestParts<ApiState> for BlobCaller {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &ApiState,
    ) -> impl Future<Output = Result<Self, ApiError>> + Send {
        std::future::ready(Self::extract(parts, state))
    }
}

impl BlobCaller {
    fn extract(parts: &Parts, state: &ApiState) -> Result<Self, ApiError> {
        let presigned = parts
            .uri
            .query()
            .is_some_and(|query| query.contains("signature="));
        if !presigned {
            return Caller::extract(parts, state).map(|Caller(context)| Self(context));
        }
        let invalid = || ApiError::from(BlobUrlRejected::Invalid);
        let Query(signature) =
            Query::<BlobSignature>::try_from_uri(&parts.uri).map_err(|_| invalid())?;
        let access = match parts.method {
            Method::GET => BlobAccess::Download,
            Method::PUT => BlobAccess::Upload,
            _ => return Err(invalid()),
        };
        let digest = parts
            .uri
            .path()
            .rsplit('/')
            .next()
            .and_then(|digest| digest.parse::<Digest>().ok())
            .ok_or_else(invalid)?;
        state
            .blob_urls
            .verify(access, digest, &signature)
            .map_err(ApiError::from)?;
        let actor = Actor::System {
            component: SystemComponent::Gateway,
        };
        Ok(Self(RequestContext::new(actor, state.ids.next())))
    }
}
