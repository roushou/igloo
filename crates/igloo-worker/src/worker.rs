use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use igloo_core::job::JobFailure;
use igloo_core::sandbox::SandboxId;
use igloo_core::worker::{Capabilities, ProtocolVersion, RuntimeKind};
use igloo_worker_protocol::v1;
use igloo_worker_protocol::v1::connect_request::Message as Uplink;
use igloo_worker_protocol::v1::connect_response::Message as Downlink;
use igloo_worker_protocol::v1::worker_gateway_service_client::WorkerGatewayServiceClient;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::transport::Endpoint;
use tonic::{Code, Request, Status, Streaming};

use crate::blobs::{BlobSource, BlobSourceError, HttpBlobSource};
use crate::config::WorkerConfig;
use crate::executor::Execution;
use crate::layers::LayerCache;
use crate::reconciler::Sandboxes;
use crate::runtime::{OciRuntime, ProcessRuntime, SandboxRuntime};
use crate::seal::{SealError, Sealer};
use crate::store::LocalStore;
use crate::usage::UsageMeter;

/// A worker: keeps one connection to the gateway, converges its sandboxes to the assignment,
/// and runs leased jobs. Running jobs and pending results survive reconnections.
pub struct Worker {
    config: WorkerConfig,
    capabilities: Capabilities,
    store: LocalStore,
    sandboxes: Sandboxes,
    runtime: Arc<dyn SandboxRuntime>,
    jobs: JoinSet<(String, Option<v1::JobResult>)>,
    running: HashMap<String, (u64, CancellationToken)>,
    deferred: Vec<v1::LeaseGrant>,
    heartbeat_every: Duration,
    usage: UsageMeter,
    usage_every: Duration,
    sealer: Sealer,
    seals: JoinSet<(String, Result<(), SealError>)>,
    sealing: HashSet<String>,
    uplink: mpsc::Sender<Uplink>,
    queued: mpsc::Receiver<Uplink>,
}

/// Why the worker cannot start or keep going.
#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    /// The data directory is unusable.
    #[error("data directory failure")]
    Io(#[from] std::io::Error),
    /// This build has no implementation of the configured runtime.
    #[error("runtime {0:?} is not available")]
    UnsupportedRuntime(RuntimeKind),
    /// The HTTP client for layer downloads could not be set up.
    #[error("blob client: {0}")]
    Blobs(#[from] BlobSourceError),
}

/// Why a connection ended.
#[derive(Debug, thiserror::Error)]
enum SessionError {
    #[error("cannot reach the gateway: {0}")]
    Connect(#[from] tonic::transport::Error),
    #[error("the gateway refused or dropped the stream: {0}")]
    Stream(#[from] Status),
    #[error("the gateway closed the stream")]
    Closed,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Worker {
    /// How often held leases are renewed at most: a third of the shortest lease granted, so a
    /// renewal can be missed twice before a lease expires.
    const HEARTBEAT: Duration = Duration::from_secs(10);
    /// How often usage is reported after the report in the hello.
    pub(crate) const USAGE_EVERY: Duration = Duration::from_secs(30);
    const MIN_HEARTBEAT: Duration = Duration::from_millis(100);
    const BACKOFF_MAX: Duration = Duration::from_secs(30);

    /// A worker for `config`, downloading layers from their presigned URLs.
    pub async fn new(config: WorkerConfig) -> Result<Self, WorkerError> {
        let source = Arc::new(HttpBlobSource::new()?);
        Self::with_blob_source(config, source).await
    }

    /// A worker for `config` reading blobs from `source`.
    pub async fn with_blob_source(
        config: WorkerConfig,
        source: Arc<dyn BlobSource>,
    ) -> Result<Self, WorkerError> {
        let store = LocalStore::open(config.data_dir.clone()).await?;
        let runtime: Arc<dyn SandboxRuntime> = match config.runtime {
            RuntimeKind::Process => Arc::new(ProcessRuntime),
            RuntimeKind::Oci => Arc::new(OciRuntime::new(
                config.oci_binary.clone(),
                store.root().join("oci"),
            )),
            RuntimeKind::Firecracker => {
                return Err(WorkerError::UnsupportedRuntime(config.runtime));
            }
        };
        let layers = LayerCache::open(
            Arc::clone(&source),
            store.root(),
            config.rootfs,
            config.layer_cache_bytes,
        )
        .await?;
        let sealer = Sealer::new(source, layers.clone(), config.rootfs, store.clone());
        let sandboxes = Sandboxes::adopt(
            Arc::clone(&runtime),
            layers.clone(),
            config.rootfs,
            store.clone(),
        )
        .await?;
        let usage = UsageMeter::new(store.root().to_owned(), layers);
        let capabilities = Capabilities::new(
            Self::os(),
            Self::arch(),
            [runtime.kind()].into(),
            ProtocolVersion::V1,
        )
        .map_err(std::io::Error::other)?;
        let (uplink, queued) = mpsc::channel(256);
        Ok(Self {
            config,
            capabilities,
            store,
            sandboxes,
            runtime,
            jobs: JoinSet::new(),
            running: HashMap::new(),
            deferred: Vec::new(),
            heartbeat_every: Self::HEARTBEAT,
            usage,
            usage_every: Self::USAGE_EVERY,
            sealer,
            seals: JoinSet::new(),
            sealing: HashSet::new(),
            uplink,
            queued,
        })
    }

    /// Reports usage every `interval` instead of every 30 seconds.
    #[cfg(test)]
    pub(crate) const fn with_usage_every(mut self, interval: Duration) -> Self {
        self.usage_every = interval;
        self
    }

    /// Serves until `cancel` fires, reconnecting with backoff whenever the connection drops.
    pub async fn run(mut self, cancel: CancellationToken) -> Result<(), WorkerError> {
        let mut backoff = Duration::from_secs(1);
        loop {
            match self.session(&cancel).await {
                Ok(()) => return Ok(()),
                Err(error) => {
                    tracing::warn!(%error, retry_in = ?backoff, "gateway connection lost");
                    if matches!(&error, SessionError::Stream(status) if Self::forgotten(status)) {
                        self.store.clear_worker_id().await?;
                    }
                }
            }
            tokio::select! {
                () = cancel.cancelled() => return Ok(()),
                () = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(Self::BACKOFF_MAX);
        }
    }

    /// The server no longer knows this worker; it must register again.
    fn forgotten(status: &Status) -> bool {
        status.code() == Code::FailedPrecondition
            && matches!(status.message(), "worker.lost" | "worker.unknown")
    }

    /// One connection: hello, welcome, resend pending results, then serve until it drops.
    async fn session(&mut self, cancel: &CancellationToken) -> Result<(), SessionError> {
        let channel = Endpoint::from_shared(self.config.server.clone())
            .map_err(SessionError::Connect)?
            .connect()
            .await?;
        let (requests, outgoing) = mpsc::channel::<v1::ConnectRequest>(256);
        let hello = self.hello().await;
        let _ = requests.send(Self::request(Uplink::Hello(hello))).await;
        let mut call = Request::new(ReceiverStream::new(outgoing));
        let bearer = format!("Bearer {}", self.config.join_token)
            .parse()
            .map_err(|_| Status::invalid_argument("join token is not valid metadata"))?;
        call.metadata_mut().insert("authorization", bearer);
        let mut responses = WorkerGatewayServiceClient::new(channel)
            .connect(call)
            .await?
            .into_inner();
        match responses.message().await? {
            Some(v1::ConnectResponse {
                message: Some(Downlink::Welcome(welcome)),
            }) => self.store.set_worker_id(&welcome.worker_id).await?,
            _ => return Err(SessionError::Closed),
        }
        for result in self.store.pending_results().await? {
            let _ = requests
                .send(Self::request(Uplink::JobResult(result)))
                .await;
        }
        self.serve(&mut responses, &requests, cancel).await
    }

    async fn serve(
        &mut self,
        responses: &mut Streaming<v1::ConnectResponse>,
        requests: &mpsc::Sender<v1::ConnectRequest>,
        cancel: &CancellationToken,
    ) -> Result<(), SessionError> {
        let mut heartbeat = tokio::time::interval(self.heartbeat_every);
        let mut usage = tokio::time::interval_at(
            tokio::time::Instant::now() + self.usage_every,
            self.usage_every,
        );
        loop {
            if heartbeat.period() != self.heartbeat_every {
                heartbeat = tokio::time::interval(self.heartbeat_every);
            }
            tokio::select! {
                () = cancel.cancelled() => return Ok(()),
                message = responses.message() => match message? {
                    Some(v1::ConnectResponse { message: Some(message) }) => {
                        self.handle(message, requests).await?;
                    }
                    Some(_) => {}
                    None => return Err(SessionError::Closed),
                },
                Some(uplink) = self.queued.recv() => {
                    if requests.send(Self::request(uplink)).await.is_err() {
                        return Err(SessionError::Closed);
                    }
                }
                Some(sealed) = self.seals.join_next(), if !self.seals.is_empty() => {
                    if let Ok((seal_id, outcome)) = sealed {
                        self.sealing.remove(&seal_id);
                        if let Err(error) = outcome {
                            tracing::warn!(seal = seal_id, %error, "sealing failed");
                            let failed = v1::SealFailed { seal_id, reason: error.to_string() };
                            let _ = requests.send(Self::request(Uplink::SealFailed(failed))).await;
                        }
                    }
                }
                Some(joined) = self.jobs.join_next(), if !self.jobs.is_empty() => {
                    if let Ok((job_id, result)) = joined {
                        self.running.remove(&job_id);
                        if let Some(result) = result {
                            self.store.save_result(&result).await?;
                            // The job's output is already queued; it must reach the server
                            // before the result that ends the job.
                            while let Ok(uplink) = self.queued.try_recv() {
                                let _ = requests.send(Self::request(uplink)).await;
                            }
                            let _ = requests.send(Self::request(Uplink::JobResult(result))).await;
                        }
                    }
                }
                Some(started) = self.sandboxes.next_started(), if self.sandboxes.has_tasks() => {
                    for report in self.sandboxes.record_started(started).await {
                        let _ = requests.send(Self::request(report)).await;
                    }
                    self.start_deferred(requests).await?;
                }
                _ = usage.tick() => {
                    let report = v1::Usage::from(&self.usage.measure(self.sandboxes.held()));
                    let _ = requests.send(Self::request(Uplink::Usage(report))).await;
                }
                _ = heartbeat.tick() => {
                    let running = self
                        .running
                        .iter()
                        .map(|(job_id, (token, _))| v1::HeldLease { job_id: job_id.clone(), token: *token });
                    let deferred = self
                        .deferred
                        .iter()
                        .map(|grant| v1::HeldLease { job_id: grant.job_id.clone(), token: grant.token });
                    let leases = running.chain(deferred).collect();
                    let _ = requests.send(Self::request(Uplink::Heartbeat(v1::Heartbeat { leases }))).await;
                }
            }
        }
    }

    async fn handle(
        &mut self,
        message: Downlink,
        requests: &mpsc::Sender<v1::ConnectRequest>,
    ) -> Result<(), SessionError> {
        match message {
            Downlink::Assignment(assignment) => {
                for report in self.sandboxes.apply(&assignment).await {
                    let _ = requests.send(Self::request(report)).await;
                }
                self.start_deferred(requests).await?;
            }
            Downlink::LeaseGrant(grant) => self.start(grant, requests).await?,
            Downlink::CancelJob(cancel) => {
                if let Some((token, running)) = self.running.get(&cancel.job_id)
                    && *token == cancel.token
                {
                    running.cancel();
                }
                self.deferred
                    .retain(|grant| grant.job_id != cancel.job_id || grant.token != cancel.token);
            }
            Downlink::ResultAck(ack) => self.store.remove_result(&ack.job_id).await?,
            Downlink::SealRequest(request) => self.seal(request),
            Downlink::Welcome(_) => {}
        }
        Ok(())
    }

    /// Starts a granted job, unless it already runs. A job whose sandbox is still starting
    /// waits for it, its lease renewed meanwhile; one whose sandbox is not here fails.
    async fn start(
        &mut self,
        grant: v1::LeaseGrant,
        requests: &mpsc::Sender<v1::ConnectRequest>,
    ) -> Result<(), SessionError> {
        if self.running.contains_key(&grant.job_id) {
            return Ok(());
        }
        let ttl = Duration::from_secs(u64::from(grant.ttl_seconds));
        if !ttl.is_zero() {
            self.heartbeat_every = self.heartbeat_every.min(ttl / 3).max(Self::MIN_HEARTBEAT);
        }
        let id = grant.sandbox_id.parse::<SandboxId>().ok();
        if id.is_some_and(|id| self.sandboxes.is_starting(id)) {
            self.deferred
                .retain(|deferred| deferred.job_id != grant.job_id);
            self.deferred.push(grant);
            return Ok(());
        }
        let sandbox = id.and_then(|id| self.sandboxes.get(id).cloned());
        let Some(sandbox) = sandbox else {
            let result = v1::JobResult {
                job_id: grant.job_id,
                token: grant.token,
                outcome: Some(v1::job_result::Outcome::Failure(
                    v1::JobFailure::from(JobFailure::SandboxUnavailable).into(),
                )),
            };
            self.store.save_result(&result).await?;
            let _ = requests
                .send(Self::request(Uplink::JobResult(result)))
                .await;
            return Ok(());
        };
        let cancel = CancellationToken::new();
        self.running
            .insert(grant.job_id.clone(), (grant.token, cancel.clone()));
        let job_id = grant.job_id.clone();
        let execution = Execution {
            grant,
            sandbox,
            runtime: Arc::clone(&self.runtime),
            uplink: self.uplink.clone(),
            cancel,
        };
        self.jobs
            .spawn(async move { (job_id, execution.run().await) });
        Ok(())
    }

    /// Seals a sandbox present here in the background, once per seal; any other seal fails.
    fn seal(&mut self, request: v1::SealRequest) {
        if !self.sealing.insert(request.seal_id.clone()) {
            return;
        }
        let present = request
            .sandbox_id
            .parse::<SandboxId>()
            .ok()
            .filter(|id| self.sandboxes.get(*id).is_some());
        let sealer = self.sealer.clone();
        self.seals.spawn(async move {
            let outcome = match present {
                Some(sandbox) => {
                    sealer
                        .seal(&request.seal_id, sandbox, &request.upload_url)
                        .await
                }
                None => Err(SealError::UnknownSandbox),
            };
            (request.seal_id, outcome)
        });
    }

    /// Starts or fails the deferred grants whose sandbox is no longer starting.
    async fn start_deferred(
        &mut self,
        requests: &mpsc::Sender<v1::ConnectRequest>,
    ) -> Result<(), SessionError> {
        for grant in std::mem::take(&mut self.deferred) {
            self.start(grant, requests).await?;
        }
        Ok(())
    }

    async fn hello(&self) -> v1::Hello {
        v1::Hello {
            worker_id: self.store.worker_id().await.unwrap_or_default(),
            protocol_version: 1,
            capabilities: Some(v1::Capabilities::from(&self.capabilities)),
            labels: self
                .config
                .labels
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
            usage: Some(v1::Usage::from(&self.usage.measure(self.sandboxes.held()))),
        }
    }

    const fn request(message: Uplink) -> v1::ConnectRequest {
        v1::ConnectRequest {
            message: Some(message),
        }
    }

    fn os() -> igloo_core::worker::Os {
        if cfg!(target_os = "macos") {
            igloo_core::worker::Os::Macos
        } else {
            igloo_core::worker::Os::Linux
        }
    }

    fn arch() -> igloo_core::worker::Arch {
        if cfg!(target_arch = "aarch64") {
            igloo_core::worker::Arch::Aarch64
        } else {
            igloo_core::worker::Arch::X86_64
        }
    }
}
