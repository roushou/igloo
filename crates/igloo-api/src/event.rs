//! The event stream: one notice per committed event.

use igloo_core::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What `GET /v1/events` sends as the `data` of each server-sent event: which resource changed
/// and when, never the domain payload. Clients refetch the resource.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[non_exhaustive]
pub struct EventNotice {
    /// The event's position in the global log; also the SSE `id`.
    pub sequence: u64,
    /// The event kind, such as `igloo.run.passed`; also the SSE `event`.
    pub kind: String,
    /// The kind of resource the event belongs to, such as `run`.
    pub resource_type: String,
    /// The resource's id, such as `run_...`.
    pub resource_id: String,
    /// The repository the resource belongs to, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// When the event was committed.
    #[schema(value_type = String, format = DateTime)]
    pub time: Timestamp,
}

impl EventNotice {
    /// A notice of event `sequence` of `kind` on `resource_type` `resource_id`, committed at
    /// `time`, for a resource of repository `repo` if it has one.
    #[must_use]
    pub fn new(
        sequence: u64,
        kind: impl Into<String>,
        resource_type: impl Into<String>,
        resource_id: impl Into<String>,
        repo: Option<String>,
        time: Timestamp,
    ) -> Self {
        Self {
            sequence,
            kind: kind.into(),
            resource_type: resource_type.into(),
            resource_id: resource_id.into(),
            repo,
            time,
        }
    }
}
