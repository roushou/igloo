//! Who the caller is.

use igloo_core::UserId;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The person a request's token stands for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct MeResource {
    /// The person's id (`usr_...`), as it appears in `owner`, `by` and `author` fields.
    pub id: String,
    /// The name to show for the person; never empty.
    pub display_name: String,
}

impl MeResource {
    /// The person `user`, shown as `display_name`.
    #[must_use]
    pub fn new(user: UserId, display_name: impl Into<String>) -> Self {
        Self {
            id: user.to_string(),
            display_name: display_name.into(),
        }
    }
}
