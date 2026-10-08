//! The worker gateway: the gRPC server end of `igloo.worker.v1`. Each worker connection runs
//! one session that pushes the worker's assignment and leases and turns its reports into
//! commands.

mod secrets;
mod session;

#[cfg(test)]
mod tests;

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use igloo_core::job::Job;
use igloo_core::sandbox::Sandbox;
use igloo_core::seal::Seal;
use igloo_core::worker::{Capabilities, WorkerId};
use igloo_core::{Actor, Labels, SystemComponent};
use igloo_worker_protocol::v1;
use igloo_worker_protocol::v1::worker_gateway_service_server::{
    WorkerGatewayService, WorkerGatewayServiceServer,
};
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

use self::secrets::JobSecrets;
use self::session::Session;
use crate::app::{
    AppError, CommandBus, InstallError, PlatformBuilder, RequestContext, TaskSpawner,
};
use crate::inbound::BlobUrls;
use crate::platform::{
    ConnectWorker, JobQueries, RegisterWorker, SandboxQueries, SealQueries, Snapshots,
};
use crate::ports::{Clock, EntityStore, EventLog, IdGenerator, IdGeneratorExt, LogStore};

/// The protocol version this server speaks.
pub const PROTOCOL_VERSION: u32 = 1;

/// The gRPC service workers connect to.
pub struct Gateway {
    shared: Arc<Shared>,
}

/// What every session needs.
struct Shared {
    bus: CommandBus,
    sandboxes: SandboxQueries,
    jobs: JobQueries,
    job_store: Arc<dyn EntityStore<Job>>,
    logs: Arc<dyn LogStore>,
    events: Arc<dyn EventLog>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
    spawner: TaskSpawner,
    join_token: Arc<str>,
    lease_ttl: Duration,
    snapshots: Snapshots,
    seals: SealQueries,
    blob_urls: BlobUrls,
    secrets: JobSecrets,
}

type ResponseStream = Pin<Box<dyn Stream<Item = Result<v1::ConnectResponse, Status>> + Send>>;

impl Gateway {
    /// How long a lease lasts unless a heartbeat renews it.
    pub const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(30);

    /// A gateway admitting workers that present `join_token`, running sessions with `spawner`
    /// and handing out layer URLs from `blob_urls`.
    pub fn new(
        platform: &PlatformBuilder,
        bus: CommandBus,
        spawner: TaskSpawner,
        join_token: impl Into<Arc<str>>,
        blob_urls: BlobUrls,
    ) -> Result<Self, InstallError> {
        let ports = platform.ports();
        let job_store = platform.store::<Job>()?;
        let sandbox_store = platform.store::<Sandbox>()?;
        Ok(Self {
            shared: Arc::new(Shared {
                bus,
                sandboxes: SandboxQueries::new(Arc::clone(&sandbox_store)),
                jobs: JobQueries::new(Arc::clone(&job_store), Arc::clone(&sandbox_store)),
                job_store,
                logs: Arc::clone(&ports.logs),
                events: Arc::clone(&ports.events),
                clock: Arc::clone(&ports.clock),
                ids: Arc::clone(&ports.ids),
                spawner,
                join_token: join_token.into(),
                lease_ttl: Self::DEFAULT_LEASE_TTL,
                snapshots: Snapshots::new(Arc::clone(&ports.blobs)),
                seals: SealQueries::new(platform.store::<Seal>()?),
                blob_urls,
                secrets: JobSecrets::new(
                    SandboxQueries::new(Arc::clone(&sandbox_store)),
                    Arc::clone(&ports.secrets),
                ),
            }),
        })
    }

    /// Grants leases lasting `ttl` unless renewed; workers heartbeat well within it.
    #[must_use]
    pub fn with_lease_ttl(mut self, ttl: Duration) -> Self {
        // A gateway that has not been turned into a service is its state's only owner.
        if let Some(shared) = Arc::get_mut(&mut self.shared) {
            shared.lease_ttl = ttl;
        }
        self
    }

    /// The tonic service to serve.
    #[must_use]
    pub fn into_service(self) -> WorkerGatewayServiceServer<Self> {
        WorkerGatewayServiceServer::new(self)
    }

    fn authenticate<T>(&self, request: &Request<T>) -> Result<(), Status> {
        let presented = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        if presented == Some(&*self.shared.join_token) {
            Ok(())
        } else {
            Err(Status::unauthenticated("invalid join token"))
        }
    }
}

impl Shared {
    /// A context for commands the gateway sends on a worker's behalf.
    fn context(&self) -> RequestContext {
        RequestContext::new(
            Actor::System {
                component: SystemComponent::Gateway,
            },
            self.ids.next(),
        )
    }

    /// Registers a new worker, or reconnects a known one.
    async fn admit(&self, hello: &v1::Hello) -> Result<WorkerId, Status> {
        if hello.protocol_version != PROTOCOL_VERSION {
            return Err(Status::failed_precondition(format!(
                "unsupported protocol version {}; this server speaks {PROTOCOL_VERSION}",
                hello.protocol_version
            )));
        }
        let capabilities = Capabilities::try_from(hello)
            .map_err(|error| Status::invalid_argument(error.to_string()))?;
        if hello.worker_id.is_empty() {
            let labels = Labels::try_from(
                hello
                    .labels
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<std::collections::BTreeMap<_, _>>(),
            )
            .map_err(|error| Status::invalid_argument(error.to_string()))?;
            let command = RegisterWorker {
                capabilities,
                labels,
            };
            return self
                .bus
                .dispatch(command, self.context())
                .await
                .map_err(status);
        }
        let worker: WorkerId = hello
            .worker_id
            .parse()
            .map_err(|_| Status::failed_precondition("worker.unknown"))?;
        let command = ConnectWorker {
            worker,
            capabilities,
        };
        self.bus
            .dispatch(command, self.context())
            .await
            .map_err(|error| match &error {
                AppError::NotFound { .. } => Status::failed_precondition("worker.unknown"),
                AppError::Domain { .. } => Status::failed_precondition("worker.lost"),
                _ => status(error),
            })?;
        Ok(worker)
    }
}

/// A command failure as a gRPC status.
fn status(error: AppError) -> Status {
    match error {
        AppError::Validation(errors) => Status::invalid_argument(errors.to_string()),
        AppError::Forbidden(denied) => Status::permission_denied(denied.to_string()),
        other => {
            tracing::error!(error = %other, "gateway command failed");
            Status::internal("internal error")
        }
    }
}

#[tonic::async_trait]
impl WorkerGatewayService for Gateway {
    type ConnectStream = ResponseStream;

    async fn connect(
        &self,
        request: Request<Streaming<v1::ConnectRequest>>,
    ) -> Result<Response<Self::ConnectStream>, Status> {
        self.authenticate(&request)?;
        let mut inbound = request.into_inner();
        let Some(v1::ConnectRequest {
            message: Some(v1::connect_request::Message::Hello(hello)),
        }) = inbound.message().await?
        else {
            return Err(Status::invalid_argument(
                "the first message must be a hello",
            ));
        };
        let worker = self.shared.admit(&hello).await?;
        let (outbound, responses) = mpsc::channel(64);
        let welcome = v1::ConnectResponse {
            message: Some(v1::connect_response::Message::Welcome(v1::Welcome {
                worker_id: worker.to_string(),
                protocol_version: PROTOCOL_VERSION,
            })),
        };
        outbound
            .send(Ok(welcome))
            .await
            .map_err(|_| Status::cancelled("worker went away"))?;
        let session = Session::new(worker, Arc::clone(&self.shared), outbound);
        self.shared.spawner.spawn("gateway-session", move |cancel| {
            session.run(inbound, cancel)
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(responses))))
    }
}
