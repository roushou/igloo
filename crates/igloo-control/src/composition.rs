use std::ffi::OsString;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use igloo_core::build::Build;
use igloo_core::change::Change;
use igloo_core::job::Job;
use igloo_core::repo::Repo;
use igloo_core::sandbox::Sandbox;
use igloo_core::seal::Seal;
use igloo_core::worker::Worker as WorkerEntity;
use igloo_core::workspace::Workspace;
use igloo_core::{Actor, Id, ValidationErrors};
use igloo_worker::{Worker, WorkerConfig, WorkerError};
use reqwest::Url;
use tokio::net::TcpListener;
use tonic::transport::Server as GrpcServer;
use tonic::transport::server::TcpIncoming;
use uuid::Uuid;

use crate::adapters::postgres::{
    PgCheckpoints, PgDatabase, PgEntityStore, PgEventLog, PgIdempotencyStore, PgLogStore,
    PgSecretStore,
};
use crate::adapters::{
    AllowAllPolicy, FsBlobStore, GitForge, OciRegistry, SystemClock, UuidV7IdGenerator,
};
use crate::agents::{AgentsModule, Task};
use crate::app::{
    AppError, ControllerSettings, InstallError, PlatformBuilder, Ports, ShutdownError,
    TaskSupervisor,
};
use crate::ci::{CiModule, Outcome, Run};
use crate::config::Config;
use crate::inbound::gateway::Gateway;
use crate::inbound::mcp::Mcp;
use crate::inbound::rest::{DevToken, RestApi};
use crate::inbound::{BlobUrls, TerminalHub, WorkspaceCredentials};
use crate::inbound::{GitHttp, GitHttpError, WebConsole};
use crate::platform::{
    BuildModule, ChangeModule, ForgeMirror, JobModule, LayerCollector, MirrorModule, RepoModule,
    SandboxModule, SealModule, SnapshotModule, WorkerModule, WorkerUsages,
};
use crate::ports::{IdGenerator, RegistryError, StorageError};
use crate::workspaces::WorkspaceModule;

/// A running server: the platform, the REST API, the worker gateway and, in development, an
/// embedded worker. The only place that names concrete adapters.
pub struct Server {
    rest: SocketAddr,
    gateway: SocketAddr,
    database: PgDatabase,
    supervisor: TaskSupervisor,
}

/// Why the server could not start.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// The database or blob storage is unavailable.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// The platform modules could not be assembled.
    #[error(transparent)]
    Install(#[from] InstallError),
    /// A listen address could not be bound.
    #[error("cannot listen: {0}")]
    Listen(#[from] std::io::Error),
    /// The image registry client could not be set up.
    #[error("image registry client: {0}")]
    Registry(#[from] RegistryError),
    /// The embedded worker could not be configured or started.
    #[error("embedded worker: {0}")]
    Worker(String),
    /// The git endpoint could not be set up.
    #[error(transparent)]
    Git(#[from] GitHttpError),
}

impl Server {
    /// The user the development token acts as.
    const DEV_USER: u128 = 1;

    /// The platform over Postgres and the file system: every port, store and module.
    async fn platform(
        config: &Config,
        database: &PgDatabase,
        events: &Arc<PgEventLog>,
        public_url: &Url,
    ) -> Result<PlatformBuilder, ServerError> {
        let ids: Arc<dyn IdGenerator> = Arc::new(UuidV7IdGenerator);
        let blobs = FsBlobStore::new(config.data_dir.join("blobs"), Arc::new(SystemClock)).await?;
        let ports = Ports {
            clock: Arc::new(SystemClock),
            ids: Arc::clone(&ids),
            events: events.clone(),
            checkpoints: Arc::new(PgCheckpoints::new(database)),
            policy: Arc::new(AllowAllPolicy),
            blobs: Arc::new(blobs),
            logs: Arc::new(PgLogStore::new(database)),
            registry: Arc::new(OciRegistry::new(config.data_dir.join("downloads"))?),
            idempotency: Arc::new(PgIdempotencyStore::new(database)),
            secrets: Arc::new(PgSecretStore::new(database, &config.secrets_key)),
            forge: Arc::new(GitForge::new(config.data_dir.join("repos"))),
        };
        let mut builder = PlatformBuilder::new(ports, config.command_timeout);
        builder.provide_store::<Sandbox>(Arc::new(PgEntityStore::new(database, Arc::clone(&ids))));
        builder.provide_store::<WorkerEntity>(Arc::new(PgEntityStore::new(
            database,
            Arc::clone(&ids),
        )));
        builder.provide_store::<Job>(Arc::new(PgEntityStore::new(database, Arc::clone(&ids))));
        builder.provide_store::<Seal>(Arc::new(PgEntityStore::new(database, Arc::clone(&ids))));
        builder.provide_store::<Repo>(Arc::new(PgEntityStore::new(database, Arc::clone(&ids))));
        builder.provide_store::<Change>(Arc::new(PgEntityStore::new(database, Arc::clone(&ids))));
        builder.provide_store::<Build>(Arc::new(PgEntityStore::new(database, Arc::clone(&ids))));
        builder.provide_store::<Run>(Arc::new(PgEntityStore::new(database, Arc::clone(&ids))));
        builder.provide_store::<Outcome>(Arc::new(PgEntityStore::new(database, Arc::clone(&ids))));
        builder.provide_store::<Task>(Arc::new(PgEntityStore::new(database, Arc::clone(&ids))));
        builder.provide_store::<Workspace>(Arc::new(PgEntityStore::new(database, ids)));
        let settings = ControllerSettings::default();
        builder.install(SandboxModule { settings })?;
        builder.install(WorkerModule { settings })?;
        builder.install(JobModule { settings })?;
        builder.install(SnapshotModule)?;
        builder.install(SealModule)?;
        builder.install(RepoModule)?;
        builder.install(ChangeModule)?;
        builder.install(MirrorModule)?;
        builder.install(BuildModule { settings })?;
        builder.install(CiModule { settings })?;
        builder.install(AgentsModule { settings })?;
        builder.install(WorkspaceModule {
            settings,
            git_base: public_url.clone(),
        })?;

        Ok(builder)
    }

    /// Connects to the database, applies migrations, assembles the platform and serves.
    pub async fn start(config: &Config) -> Result<Self, ServerError> {
        let database = PgDatabase::connect(&config.database_url).await?;
        let events = PgEventLog::new(&database).await?;
        let rest_listener = TcpListener::bind(config.listen).await?;
        let gateway_listener = TcpListener::bind(config.gateway_listen).await?;
        let public_url = match &config.public_url {
            Some(url) => url.clone(),
            None => format!("http://{}/", rest_listener.local_addr()?)
                .parse()
                .map_err(std::io::Error::other)?,
        };
        let builder = Self::platform(config, &database, &events, &public_url).await?;
        let blob_urls = BlobUrls::new(
            public_url,
            config.blob_key.clone(),
            Arc::clone(&builder.ports().clock),
        );

        let supervisor = TaskSupervisor::new();
        supervisor.spawn("event-log", move |cancel| async move {
            events.follow(cancel).await.map_err(AppError::from)
        });
        let usages = WorkerUsages::new();
        let terminals = TerminalHub::new();
        let gateway = Gateway::new(
            &builder,
            builder.bus(),
            supervisor.spawner(),
            config.join_token.clone(),
            blob_urls.clone(),
        )?
        .with_lease_ttl(config.lease_ttl)
        .with_usages(usages.clone())
        .with_terminals(terminals.clone());
        let actor = Actor::Human {
            user: Id::from_uuid(Uuid::from_u128(Self::DEV_USER)),
        };
        let collector = LayerCollector::new(&builder, builder.bus())?;
        let mirror = ForgeMirror::new(&builder)?;
        let token = DevToken::new(config.dev_token.clone(), actor);
        let git = GitHttp::new(
            &builder,
            builder.bus(),
            token.clone(),
            WorkspaceCredentials::new(&blob_urls),
            config.data_dir.join("repos"),
            supervisor.spawner(),
        )
        .await?;
        let rest = RestApi::new(&builder, builder.bus(), token, blob_urls)?
            .with_usages(usages)
            .with_terminals(terminals)
            .with_collector(collector.clone());
        let api = Mcp::router(&rest).merge(rest.router()).merge(git.router());
        let http = match &config.web_dir {
            Some(dir) => WebConsole::new(dir).mount(api),
            None => api,
        };
        let _bus = builder.build().start(&supervisor);
        supervisor.spawn("layer-collector", |cancel| collector.run(cancel));
        let mirror_interval = config.mirror_interval;
        supervisor.spawn("forge-mirror", move |cancel| {
            mirror.run(mirror_interval, cancel)
        });

        let server = Self {
            rest: rest_listener.local_addr()?,
            gateway: gateway_listener.local_addr()?,
            database,
            supervisor,
        };
        server.supervisor.spawn("rest", move |cancel| async move {
            axum::serve(rest_listener, http)
                .with_graceful_shutdown(cancel.cancelled_owned())
                .await
                .map_err(AppError::infrastructure)
        });
        server
            .supervisor
            .spawn("gateway", move |cancel| async move {
                GrpcServer::builder()
                    .add_service(gateway.into_service())
                    .serve_with_incoming_shutdown(
                        TcpIncoming::from(gateway_listener),
                        cancel.cancelled_owned(),
                    )
                    .await
                    .map_err(AppError::infrastructure)
            });
        if config.embedded_worker {
            server.start_embedded_worker(config).await?;
        }
        Ok(server)
    }

    /// Runs a worker in this process with the unisolated process runtime.
    async fn start_embedded_worker(&self, config: &Config) -> Result<(), ServerError> {
        let worker_config = WorkerConfig::from_vars([
            (
                "IGLOO_WORKER_SERVER",
                OsString::from(format!("http://{}", self.gateway)),
            ),
            ("IGLOO_WORKER_JOIN_TOKEN", config.join_token.clone().into()),
            (
                "IGLOO_WORKER_DATA_DIR",
                config.data_dir.join("worker").into(),
            ),
            ("IGLOO_WORKER_RUNTIME", "process".into()),
            ("IGLOO_WORKER_DEV_MODE", "true".into()),
            ("IGLOO_WORKER_LABELS", "embedded=true".into()),
        ])
        .map_err(|errors: ValidationErrors| ServerError::Worker(errors.to_string()))?;
        let worker = Worker::new(worker_config)
            .await
            .map_err(|error: WorkerError| ServerError::Worker(error.to_string()))?;
        self.supervisor
            .spawn("embedded-worker", move |cancel| async move {
                worker.run(cancel).await.map_err(AppError::infrastructure)
            });
        Ok(())
    }

    /// Where the REST API listens.
    #[must_use]
    pub const fn rest_address(&self) -> SocketAddr {
        self.rest
    }

    /// Where workers connect.
    #[must_use]
    pub const fn gateway_address(&self) -> SocketAddr {
        self.gateway
    }

    /// The database, for inspection.
    #[must_use]
    pub const fn database(&self) -> &PgDatabase {
        &self.database
    }

    /// Stops serving, waiting up to `grace` for in-flight work.
    pub async fn shutdown(self, grace: Duration) -> Result<(), ShutdownError> {
        self.supervisor.shutdown(grace).await
    }
}
