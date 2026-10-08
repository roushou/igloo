use std::io;

use axum::extract::{FromRequest, Request};
use axum::http::header::CONTENT_LENGTH;
use futures_util::StreamExt;
use tokio_util::io::StreamReader;

use igloo_api::problem::Problem;

use super::{ApiError, ApiState};
use crate::ports::BlobReader;

/// A request body read as a stream of at most the API's blob limit, never buffered whole. A
/// declared length over the limit is refused with 413 before reading; a body growing past the
/// limit fails while it streams.
pub(super) struct Upload(pub(super) BlobReader);

impl FromRequest<ApiState> for Upload {
    type Rejection = ApiError;

    fn from_request(
        request: Request,
        state: &ApiState,
    ) -> impl Future<Output = Result<Self, ApiError>> + Send {
        std::future::ready(Self::read(request, state.max_blob_bytes))
    }
}

impl Upload {
    fn read(request: Request, limit: u64) -> Result<Self, ApiError> {
        let declared = request
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|length| length.to_str().ok())
            .and_then(|length| length.parse::<u64>().ok());
        if declared.is_some_and(|length| length > limit) {
            return Err(ApiError(Box::new(
                Problem::new(413, "request.body", "Request body too large")
                    .with_detail(format!("the limit is {limit} bytes")),
            )));
        }
        let mut remaining = limit;
        let chunks = request.into_body().into_data_stream().map(move |chunk| {
            let chunk = chunk.map_err(io::Error::other)?;
            remaining = u64::try_from(chunk.len())
                .ok()
                .and_then(|length| remaining.checked_sub(length))
                .ok_or_else(|| io::Error::other("the upload exceeds the size limit"))?;
            Ok::<_, io::Error>(chunk)
        });
        Ok(Self(Box::pin(StreamReader::new(chunks))))
    }
}
