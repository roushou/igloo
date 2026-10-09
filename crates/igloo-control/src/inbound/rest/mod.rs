//! The REST API (`/v1`). Routes and their OpenAPI description are registered together, so the
//! published document always matches what is served.

mod auth;
mod blobs;
pub(super) mod changes;
mod events;
mod idempotency;
mod jobs;
mod problem;
pub(super) mod repos;
pub(super) mod runs;
mod sandboxes;
mod seals;
pub(super) mod tasks;
mod upload;
mod workers;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use axum::Router;
use igloo_core::build::Build;
use igloo_core::change::Change;
use igloo_core::job::Job;
use igloo_core::repo::Repo;
use igloo_core::sandbox::Sandbox;
use igloo_core::seal::Seal;
use igloo_core::worker::Worker;
use utoipa::openapi::OpenApi as Document;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use self::events::EventRepos;
use self::idempotency::Idempotency;
use crate::agents::{Task, TaskQueries, Transcripts};
use crate::app::{CommandBus, InstallError, PlatformBuilder};
use crate::ci::{Merger, Run, RunQueries};
use crate::inbound::BlobUrls;
use crate::platform::{
    BuildQueries, ChangeHeads, ChangeQueries, ImageImporter, RepoQueries, RepoSnapshots,
    SandboxQueries, Snapshots, WorkerUsages,
};
use crate::ports::{
    BlobStore, Clock, EntityStore, EventLog, Forge, IdGenerator, IdempotencyStore, LogStore,
    SecretStore,
};

pub use auth::DevToken;
pub use problem::ApiError;

/// What every handler needs: the bus for writes and queries for reads. The MCP server shares it.
#[derive(Clone)]
pub(crate) struct ApiState {
    bus: CommandBus,
    sandboxes: SandboxQueries,
    jobs: Arc<dyn EntityStore<Job>>,
    seals: Arc<dyn EntityStore<Seal>>,
    workers: Arc<dyn EntityStore<Worker>>,
    usages: WorkerUsages,
    repos: RepoQueries,
    changes: ChangeQueries,
    change_heads: ChangeHeads,
    runs: RunQueries,
    builds: BuildQueries,
    tasks: TaskQueries,
    transcripts: Transcripts,
    merger: Merger,
    repo_snapshots: RepoSnapshots,
    forge: Arc<dyn Forge>,
    secrets: Arc<dyn SecretStore>,
    snapshots: Snapshots,
    importer: ImageImporter,
    blobs: Arc<dyn BlobStore>,
    logs: Arc<dyn LogStore>,
    events: Arc<dyn EventLog>,
    event_repos: EventRepos,
    pub(super) ids: Arc<dyn IdGenerator>,
    pub(super) auth: DevToken,
    blob_urls: BlobUrls,
    idempotency: Arc<dyn IdempotencyStore>,
    clock: Arc<dyn Clock>,
    max_blob_bytes: u64,
}

/// The REST API over a built platform.
pub struct RestApi {
    state: ApiState,
}

#[derive(OpenApi)]
#[openapi(
    info(title = "Igloo", version = "v1", description = "The Igloo control plane API."),
    components(schemas(igloo_api::list::ListOrder)),
    modifiers(&BearerAuth),
    security(("bearer" = []))
)]
struct ApiDoc;

struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut Document) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer",
                SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()),
            );
        }
    }
}

impl RestApi {
    /// The default limit on uploaded blob size: 64 GiB.
    pub const DEFAULT_MAX_BLOB_BYTES: u64 = 64 * 1024 * 1024 * 1024;

    /// An API dispatching through `bus`, reading from the stores `platform` was given,
    /// authenticating with `auth` or with presigned URLs from `blob_urls`.
    pub fn new(
        platform: &PlatformBuilder,
        bus: CommandBus,
        auth: DevToken,
        blob_urls: BlobUrls,
    ) -> Result<Self, InstallError> {
        let ports = platform.ports();
        Ok(Self {
            state: ApiState {
                bus: bus.clone(),
                sandboxes: SandboxQueries::new(platform.store::<Sandbox>()?),
                jobs: platform.store::<Job>()?,
                seals: platform.store::<Seal>()?,
                workers: platform.store::<Worker>()?,
                usages: WorkerUsages::new(),
                repos: RepoQueries::new(platform.store::<Repo>()?, Arc::clone(&ports.secrets)),
                changes: ChangeQueries::new(platform.store::<Change>()?),
                runs: RunQueries::new(platform.store::<Run>()?),
                builds: BuildQueries::new(platform.store::<Build>()?),
                tasks: TaskQueries::new(platform.store::<Task>()?),
                transcripts: Transcripts::new(Arc::clone(&ports.logs)),
                merger: Merger::new(
                    RepoQueries::new(platform.store::<Repo>()?, Arc::clone(&ports.secrets)),
                    RunQueries::new(platform.store::<Run>()?),
                    Arc::clone(&ports.forge),
                    bus,
                ),
                change_heads: ChangeHeads::new(
                    RepoQueries::new(platform.store::<Repo>()?, Arc::clone(&ports.secrets)),
                    Arc::clone(&ports.forge),
                ),
                repo_snapshots: RepoSnapshots::new(
                    Arc::clone(&ports.forge),
                    Arc::clone(&ports.blobs),
                ),
                forge: Arc::clone(&ports.forge),
                secrets: Arc::clone(&ports.secrets),
                snapshots: Snapshots::new(Arc::clone(&ports.blobs)),
                importer: ImageImporter::new(Arc::clone(&ports.registry), Arc::clone(&ports.blobs)),
                blobs: Arc::clone(&ports.blobs),
                logs: Arc::clone(&ports.logs),
                events: Arc::clone(&ports.events),
                event_repos: EventRepos::new(platform)?,
                ids: Arc::clone(&ports.ids),
                auth,
                blob_urls,
                idempotency: Arc::clone(&ports.idempotency),
                clock: Arc::clone(&ports.clock),
                max_blob_bytes: Self::DEFAULT_MAX_BLOB_BYTES,
            },
        })
    }

    /// Reads worker usage reports from `usages`, the ones a gateway given the same handle keeps.
    #[must_use]
    pub fn with_usages(mut self, usages: WorkerUsages) -> Self {
        self.state.usages = usages;
        self
    }

    /// Limits uploaded blobs to `bytes`.
    #[must_use]
    pub const fn with_max_blob_bytes(mut self, bytes: u64) -> Self {
        self.state.max_blob_bytes = bytes;
        self
    }

    /// The axum router serving every `/v1` route.
    pub fn router(self) -> Router {
        let guard = axum::middleware::from_fn_with_state(self.state.clone(), Idempotency::guard);
        let (router, _) = Self::routes().with_state(self.state).split_for_parts();
        router.layer(guard)
    }

    /// The state handlers read from.
    pub(super) const fn state(&self) -> &ApiState {
        &self.state
    }

    /// The OpenAPI document of every route.
    #[must_use]
    pub fn openapi() -> Document {
        Self::routes().split_for_parts().1
    }

    fn routes() -> OpenApiRouter<ApiState> {
        OpenApiRouter::with_openapi(ApiDoc::openapi())
            .routes(routes!(sandboxes::create, sandboxes::list))
            .routes(routes!(sandboxes::get))
            .routes(routes!(workers::list))
            .routes(routes!(sandboxes::stop))
            .routes(routes!(sandboxes::exec))
            .routes(routes!(jobs::get))
            .routes(routes!(jobs::logs))
            .routes(routes!(blobs::put, blobs::get))
            .routes(routes!(blobs::create_snapshot))
            .routes(routes!(blobs::import_image))
            .routes(routes!(blobs::get_snapshot))
            .routes(routes!(seals::create))
            .routes(routes!(seals::get))
            .routes(routes!(seals::upload))
            .routes(routes!(repos::register, repos::list))
            .routes(routes!(repos::get))
            .routes(routes!(repos::list_secrets))
            .routes(routes!(repos::snapshot))
            .routes(routes!(changes::open, changes::list))
            .routes(routes!(changes::get))
            .routes(routes!(changes::diff))
            .routes(routes!(changes::revise))
            .routes(routes!(changes::close))
            .routes(routes!(changes::approve))
            .routes(routes!(changes::comment))
            .routes(routes!(changes::request_changes))
            .routes(routes!(changes::merge))
            .routes(routes!(runs::of_change))
            .routes(routes!(runs::of_repo))
            .routes(routes!(runs::get))
            .routes(routes!(repos::set_secret, repos::delete_secret))
            .routes(routes!(tasks::create, tasks::list))
            .routes(routes!(tasks::get))
            .routes(routes!(tasks::cancel))
            .routes(routes!(tasks::transcript))
            .routes(routes!(events::stream))
    }
}
