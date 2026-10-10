use axum::Json;
use axum::extract::State;
use igloo_api::me::MeResource;
use igloo_api::problem::Problem;

use super::auth::Caller;
use super::{ApiError, ApiState};

/// Says who the caller is: the person the token stands for (for an agent, the person it acts
/// for), so a client can tell which `owner`, `by` or `author` is the viewer.
#[utoipa::path(
    get,
    operation_id = "getMe",
    path = "/v1/me",
    tag = "me",
    responses((status = 200, body = MeResource), (status = 404, body = Problem))
)]
pub(super) async fn get(
    State(state): State<ApiState>,
    Caller(context): Caller,
) -> Result<Json<MeResource>, ApiError> {
    let user = context
        .actor
        .principal()
        .ok_or_else(|| ApiError::not_found("me.not_a_person"))?;
    Ok(Json(MeResource::new(user, state.auth.display_name())))
}
