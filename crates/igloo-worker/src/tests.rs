//! The worker against a scripted gateway and an in-memory blob source.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use igloo_core::sandbox::Sandbox;
use igloo_core::snapshot::{MediaType, SnapshotLayer};
use igloo_core::{Digest, Id};
use igloo_worker_protocol::v1;
use igloo_worker_protocol::v1::connect_request::Message as Uplink;
use igloo_worker_protocol::v1::connect_response::Message as Downlink;
use igloo_worker_protocol::v1::worker_gateway_service_server::{
    WorkerGatewayService, WorkerGatewayServiceServer,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::transport::Server;
use tonic::transport::server::TcpIncoming;
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;

use crate::{BlobSource, BlobSourceError, RemoteLayer, Worker, WorkerConfig};

const WORKER_ID: &str = "wrk_00000000000000000000000001";

/// Blobs in memory, counting downloads.
struct MemoryBlobs {
    blobs: Mutex<HashMap<Digest, Vec<u8>>>,
    fetches: AtomicUsize,
    /// Every upload: its URL and bytes.
    uploads: Mutex<Vec<(reqwest::Url, Vec<u8>)>>,
    /// Downloads of these digests wait until `gate` is closed.
    gated: Mutex<HashSet<Digest>>,
    gate: Semaphore,
}

impl Default for MemoryBlobs {
    fn default() -> Self {
        Self {
            blobs: Mutex::default(),
            fetches: AtomicUsize::default(),
            uploads: Mutex::default(),
            gated: Mutex::default(),
            gate: Semaphore::new(0),
        }
    }
}

impl MemoryBlobs {
    fn put(&self, bytes: Vec<u8>) -> Digest {
        let digest = Digest::from_blake3(*blake3::hash(&bytes).as_bytes());
        self.blobs.lock().expect("blobs").insert(digest, bytes);
        digest
    }

    /// A snapshot whose one layer only downloads once `gate` is closed.
    fn gated_snapshot(&self) -> Vec<v1::SnapshotLayer> {
        let mut layer = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(5);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        layer
            .append_data(&mut header, "workspace/slow.txt", b"slow\n".as_slice())
            .expect("append");
        let digest = self.put(layer.into_inner().expect("tar"));
        self.gated.lock().expect("gated").insert(digest);
        let layer = SnapshotLayer::new(digest, MediaType::Tar);
        vec![v1::SnapshotLayer::new(
            &layer,
            format!("http://blobs.test/{digest}"),
        )]
    }

    /// A snapshot with one layer holding `workspace/hello.txt`; returns its layers.
    fn snapshot(&self) -> Vec<v1::SnapshotLayer> {
        let mut layer = tar::Builder::new(Vec::new());
        let contents = b"hi\n";
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        layer
            .append_data(&mut header, "workspace/hello.txt", contents.as_slice())
            .expect("append");
        let digest = self.put(layer.into_inner().expect("tar"));
        let layer = SnapshotLayer::new(digest, MediaType::Tar);
        vec![v1::SnapshotLayer::new(
            &layer,
            format!("http://blobs.test/{digest}"),
        )]
    }
}

#[async_trait]
impl BlobSource for MemoryBlobs {
    async fn download(
        &self,
        layer: &RemoteLayer,
        file: &mut tokio::fs::File,
    ) -> Result<(), BlobSourceError> {
        self.fetches.fetch_add(1, Ordering::Relaxed);
        let digest = layer.layer().digest();
        let gated = self.gated.lock().expect("gated").contains(&digest);
        if gated {
            let _closed = self.gate.acquire().await;
        }
        let bytes = self
            .blobs
            .lock()
            .expect("blobs")
            .get(&digest)
            .cloned()
            .ok_or(BlobSourceError::NotFound(digest))?;
        file.write_all(&bytes)
            .await
            .map_err(|error| BlobSourceError::Transport(Box::new(error)))?;
        file.flush()
            .await
            .map_err(|error| BlobSourceError::Transport(Box::new(error)))
    }

    async fn upload(
        &self,
        url: &reqwest::Url,
        mut file: tokio::fs::File,
    ) -> Result<(), BlobSourceError> {
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .await
            .map_err(|error| BlobSourceError::Transport(Box::new(error)))?;
        self.uploads
            .lock()
            .expect("uploads")
            .push((url.clone(), bytes));
        Ok(())
    }
}

/// One accepted connection, driven by the test.
struct Session {
    hello: v1::Hello,
    inbound: Streaming<v1::ConnectRequest>,
    outbound: mpsc::Sender<Result<v1::ConnectResponse, Status>>,
}

/// Welcomes every worker as `WORKER_ID` and hands the connection to the test.
struct FakeGateway {
    sessions: mpsc::Sender<Session>,
}

#[tonic::async_trait]
impl WorkerGatewayService for FakeGateway {
    type ConnectStream = ReceiverStream<Result<v1::ConnectResponse, Status>>;

    async fn connect(
        &self,
        request: Request<Streaming<v1::ConnectRequest>>,
    ) -> Result<Response<Self::ConnectStream>, Status> {
        let mut inbound = request.into_inner();
        let Some(v1::ConnectRequest {
            message: Some(Uplink::Hello(hello)),
        }) = inbound.message().await?
        else {
            return Err(Status::invalid_argument("hello first"));
        };
        let (outbound, responses) = mpsc::channel(64);
        let welcome = v1::Welcome {
            worker_id: WORKER_ID.to_owned(),
            protocol_version: 1,
        };
        let _ = outbound
            .send(Ok(v1::ConnectResponse {
                message: Some(Downlink::Welcome(welcome)),
            }))
            .await;
        let _ = self
            .sessions
            .send(Session {
                hello,
                inbound,
                outbound,
            })
            .await;
        Ok(Response::new(ReceiverStream::new(responses)))
    }
}

impl Session {
    async fn send(&self, message: Downlink) {
        self.outbound
            .send(Ok(v1::ConnectResponse {
                message: Some(message),
            }))
            .await
            .expect("send");
    }

    /// The next worker message other than heartbeats and usage reports.
    async fn next(&mut self) -> Uplink {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(10), self.inbound.message())
                .await
                .expect("a worker message within 10 s")
                .expect("stream healthy")
                .expect("stream open")
                .message
                .expect("message set");
            if !matches!(message, Uplink::Heartbeat(_) | Uplink::Usage(_)) {
                return message;
            }
        }
    }

    /// Skips worker messages until `pick` accepts one.
    async fn until<T>(&mut self, mut pick: impl FnMut(Uplink) -> Option<T>) -> T {
        loop {
            if let Some(found) = pick(self.next().await) {
                return found;
            }
        }
    }

    async fn assign(&self, sandbox: &str, layers: &[v1::SnapshotLayer], desired: v1::DesiredState) {
        let mut assigned = v1::AssignedSandbox {
            sandbox_id: sandbox.to_owned(),
            snapshot: Digest::from_blake3([7; 32]).to_string(),
            layers: layers.to_vec(),
            generation: 1,
            millicpus: 1000,
            memory_mib: 2048,
            ..v1::AssignedSandbox::default()
        };
        assigned.set_desired(desired);
        self.send(Downlink::Assignment(v1::Assignment {
            sandboxes: vec![assigned],
        }))
        .await;
    }

    /// Assigns every `(sandbox, layers)` to run.
    async fn assign_all(&self, sandboxes: &[(&str, &[v1::SnapshotLayer])]) {
        let sandboxes = sandboxes
            .iter()
            .map(|(sandbox, layers)| {
                let mut assigned = v1::AssignedSandbox {
                    sandbox_id: (*sandbox).to_owned(),
                    snapshot: Digest::from_blake3([7; 32]).to_string(),
                    layers: layers.to_vec(),
                    generation: 1,
                    millicpus: 1000,
                    memory_mib: 2048,
                    ..v1::AssignedSandbox::default()
                };
                assigned.set_desired(v1::DesiredState::Running);
                assigned
            })
            .collect();
        self.send(Downlink::Assignment(v1::Assignment { sandboxes }))
            .await;
    }

    async fn phase(&mut self) -> v1::SandboxPhase {
        self.until(|message| match message {
            Uplink::SandboxStatus(status) => Some(status.phase()),
            _ => None,
        })
        .await
    }

    async fn grant(&self, job: &str, sandbox: &str, argv: &[&str], timeout_seconds: u32) {
        self.send(Downlink::LeaseGrant(v1::LeaseGrant {
            job_id: job.to_owned(),
            token: 1,
            ttl_seconds: 30,
            sandbox_id: sandbox.to_owned(),
            argv: argv.iter().map(ToString::to_string).collect(),
            env: HashMap::new(),
            timeout_seconds,
        }))
        .await;
    }

    async fn result(&mut self) -> v1::JobResult {
        self.until(|message| match message {
            Uplink::JobResult(result) => Some(result),
            _ => None,
        })
        .await
    }
}

struct Harness {
    address: SocketAddr,
    sessions: mpsc::Receiver<Session>,
    blobs: Arc<MemoryBlobs>,
    tasks: JoinSet<()>,
}

impl Harness {
    async fn start() -> Self {
        let (sessions_tx, sessions) = mpsc::channel(4);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let mut tasks = JoinSet::new();
        let service = WorkerGatewayServiceServer::new(FakeGateway {
            sessions: sessions_tx,
        });
        tasks.spawn(async move {
            let _ = Server::builder()
                .add_service(service)
                .serve_with_incoming(TcpIncoming::from(listener))
                .await;
        });
        Self {
            address,
            sessions,
            blobs: Arc::new(MemoryBlobs::default()),
            tasks,
        }
    }

    /// Runs a worker on `data_dir` until the returned token is cancelled.
    async fn worker(&mut self, data_dir: &Path) -> CancellationToken {
        self.worker_reporting_every(data_dir, Worker::USAGE_EVERY)
            .await
    }

    /// As [`Harness::worker`], reporting usage every `interval`.
    async fn worker_reporting_every(
        &mut self,
        data_dir: &Path,
        interval: Duration,
    ) -> CancellationToken {
        let overlay = cfg!(target_os = "linux") && std::env::var_os("IGLOO_TEST_OVERLAY").is_some();
        let config = WorkerConfig::from_vars([
            (
                "IGLOO_WORKER_SERVER",
                OsString::from(format!("http://{}", self.address)),
            ),
            ("IGLOO_WORKER_JOIN_TOKEN", "join".into()),
            ("IGLOO_WORKER_DATA_DIR", data_dir.into()),
            ("IGLOO_WORKER_RUNTIME", "process".into()),
            ("IGLOO_WORKER_DEV_MODE", "true".into()),
            ("IGLOO_WORKER_OVERLAY", overlay.to_string().into()),
        ])
        .expect("config");
        let worker = Worker::with_blob_source(config, self.blobs.clone())
            .await
            .expect("worker");
        let worker = worker.with_usage_every(interval);
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        self.tasks.spawn(async move {
            let _ = worker.run(token).await;
        });
        cancel
    }

    async fn session(&mut self) -> Session {
        tokio::time::timeout(Duration::from_secs(10), self.sessions.recv())
            .await
            .expect("a connection within 10 s")
            .expect("gateway running")
    }
}

fn sandbox_id() -> String {
    Id::<Sandbox>::from_uuid(Uuid::from_u128(5)).to_string()
}

const JOB: &str = "job_00000000000000000000000009";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_job_runs_in_its_materialized_sandbox() {
    let mut harness = Harness::start().await;
    let data = tempfile::tempdir().expect("data dir");
    let _worker = harness.worker(data.path()).await;
    let mut session = harness.session().await;
    assert_eq!(session.hello.worker_id, "", "a new worker registers");

    let snapshot = harness.blobs.snapshot();
    session
        .assign(&sandbox_id(), &snapshot, v1::DesiredState::Running)
        .await;
    assert_eq!(session.phase().await, v1::SandboxPhase::Starting);
    assert_eq!(session.phase().await, v1::SandboxPhase::Running);

    session
        .grant(JOB, &sandbox_id(), &["cat", "hello.txt"], 30)
        .await;
    let mut output = Vec::new();
    let result = session
        .until(|message| match message {
            Uplink::LogChunk(chunk) => {
                output.extend(chunk.data);
                None
            }
            Uplink::JobResult(result) => Some(result),
            _ => None,
        })
        .await;
    assert_eq!(output, b"hi\n");
    assert_eq!(result.outcome, Some(v1::job_result::Outcome::ExitCode(0)));

    session
        .grant(
            "job_0000000000000000000000000a",
            &sandbox_id(),
            &["sleep", "5"],
            1,
        )
        .await;
    let timed_out = session.result().await;
    assert_eq!(
        timed_out.outcome,
        Some(v1::job_result::Outcome::Failure(
            v1::JobFailure::TimedOut.into()
        ))
    );

    session
        .assign(&sandbox_id(), &snapshot, v1::DesiredState::Stopped)
        .await;
    assert_eq!(session.phase().await, v1::SandboxPhase::Stopping);
    assert_eq!(session.phase().await, v1::SandboxPhase::Stopped);
    assert!(!data.path().join("sandboxes").join(sandbox_id()).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn results_are_resent_until_acknowledged() {
    let mut harness = Harness::start().await;
    let data = tempfile::tempdir().expect("data dir");
    let _worker = harness.worker(data.path()).await;
    let mut session = harness.session().await;
    let snapshot = harness.blobs.snapshot();
    session
        .assign(&sandbox_id(), &snapshot, v1::DesiredState::Running)
        .await;
    session.grant(JOB, &sandbox_id(), &["true"], 30).await;
    let first = session.result().await;
    drop(session);

    let mut session = harness.session().await;
    assert_eq!(
        session.hello.worker_id, WORKER_ID,
        "the worker keeps its id"
    );
    let resent = session.result().await;
    assert_eq!(resent, first);
    session
        .send(Downlink::ResultAck(v1::ResultAck {
            job_id: JOB.to_owned(),
            token: 1,
            accepted: true,
            reason: String::new(),
        }))
        .await;
    let outbox = data.path().join("outbox");
    for _ in 0..100 {
        if std::fs::read_dir(&outbox).expect("outbox").next().is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the acknowledged result must leave the outbox");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restarted_worker_adopts_its_sandboxes() {
    let mut harness = Harness::start().await;
    let data = tempfile::tempdir().expect("data dir");
    let first = harness.worker(data.path()).await;
    let mut session = harness.session().await;
    let snapshot = harness.blobs.snapshot();
    session
        .assign(&sandbox_id(), &snapshot, v1::DesiredState::Running)
        .await;
    assert_eq!(session.phase().await, v1::SandboxPhase::Starting);
    assert_eq!(session.phase().await, v1::SandboxPhase::Running);
    let fetches = harness.blobs.fetches.load(Ordering::Relaxed);
    first.cancel();
    drop(session);

    let _second = harness.worker(data.path()).await;
    let mut session = harness.session().await;
    session
        .assign(&sandbox_id(), &snapshot, v1::DesiredState::Running)
        .await;
    assert_eq!(
        session.phase().await,
        v1::SandboxPhase::Running,
        "adopted, not restarted"
    );
    assert_eq!(harness.blobs.fetches.load(Ordering::Relaxed), fetches);
    session
        .grant(JOB, &sandbox_id(), &["cat", "hello.txt"], 30)
        .await;
    assert_eq!(
        session.result().await.outcome,
        Some(v1::job_result::Outcome::ExitCode(0))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_job_is_stopped_without_a_result() {
    let mut harness = Harness::start().await;
    let data = tempfile::tempdir().expect("data dir");
    let _worker = harness.worker(data.path()).await;
    let mut session = harness.session().await;
    let snapshot = harness.blobs.snapshot();
    session
        .assign(&sandbox_id(), &snapshot, v1::DesiredState::Running)
        .await;
    session
        .grant(JOB, &sandbox_id(), &["sleep", "30"], 60)
        .await;
    session
        .until(|message| matches!(message, Uplink::JobStarted(_)).then_some(()))
        .await;
    session
        .send(Downlink::CancelJob(v1::CancelJob {
            job_id: JOB.to_owned(),
            token: 1,
        }))
        .await;
    session
        .grant(
            "job_0000000000000000000000000b",
            &sandbox_id(),
            &["true"],
            30,
        )
        .await;
    let next = session.result().await;
    assert_eq!(
        next.job_id, "job_0000000000000000000000000b",
        "no result for the cancelled job"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_layer_that_cannot_be_downloaded_fails_the_sandbox() {
    let mut harness = Harness::start().await;
    let data = tempfile::tempdir().expect("data dir");
    let _worker = harness.worker(data.path()).await;
    let mut session = harness.session().await;
    let missing = SnapshotLayer::new(Digest::from_blake3([9; 32]), MediaType::Tar);
    let layers = [v1::SnapshotLayer::new(
        &missing,
        "http://blobs.test/missing",
    )];
    session
        .assign(&sandbox_id(), &layers, v1::DesiredState::Running)
        .await;
    assert_eq!(session.phase().await, v1::SandboxPhase::Starting);
    assert_eq!(session.phase().await, v1::SandboxPhase::Failed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sandboxes_of_one_snapshot_share_layers_but_not_writes() {
    let mut harness = Harness::start().await;
    let data = tempfile::tempdir().expect("data dir");
    let _worker = harness.worker(data.path()).await;
    let mut session = harness.session().await;
    let snapshot = harness.blobs.snapshot();
    let first = sandbox_id();
    let second = Id::<Sandbox>::from_uuid(Uuid::from_u128(6)).to_string();
    session
        .assign_all(&[(&first, &snapshot), (&second, &snapshot)])
        .await;
    for _ in 0..2 {
        session
            .until(|message| match message {
                Uplink::SandboxStatus(status) if status.phase() == v1::SandboxPhase::Running => {
                    Some(())
                }
                _ => None,
            })
            .await;
    }
    assert_eq!(harness.blobs.fetches.load(Ordering::Relaxed), 1);

    session
        .grant(JOB, &first, &["sh", "-c", "echo mine > hello.txt"], 30)
        .await;
    assert_eq!(
        session.result().await.outcome,
        Some(v1::job_result::Outcome::ExitCode(0))
    );
    session
        .grant(
            "job_0000000000000000000000000b",
            &second,
            &["sh", "-c", "grep -q hi hello.txt"],
            30,
        )
        .await;
    assert_eq!(
        session.result().await.outcome,
        Some(v1::job_result::Outcome::ExitCode(0)),
        "the second sandbox does not see the first one's write"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn work_continues_while_a_sandbox_starts_and_its_jobs_wait_for_it() {
    let mut harness = Harness::start().await;
    let data = tempfile::tempdir().expect("data dir");
    let _worker = harness.worker(data.path()).await;
    let mut session = harness.session().await;
    let slow = sandbox_id();
    let fast = Id::<Sandbox>::from_uuid(Uuid::from_u128(6)).to_string();
    let slow_layers = harness.blobs.gated_snapshot();
    let fast_layers = harness.blobs.snapshot();
    session
        .assign_all(&[(&slow, &slow_layers), (&fast, &fast_layers)])
        .await;
    session.grant(JOB, &slow, &["cat", "slow.txt"], 30).await;
    session
        .grant("job_0000000000000000000000000b", &fast, &["true"], 30)
        .await;
    let first = session.result().await;
    assert_eq!(
        first.job_id, "job_0000000000000000000000000b",
        "the fast sandbox's job finishes while the slow one starts"
    );

    harness.blobs.gate.close();
    let deferred = session.result().await;
    assert_eq!(deferred.job_id, JOB);
    assert_eq!(
        deferred.outcome,
        Some(v1::job_result::Outcome::ExitCode(0)),
        "the deferred job runs once its sandbox is up"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seal_uploads_the_sandbox_file_system_and_an_unknown_one_fails() {
    let mut harness = Harness::start().await;
    let data = tempfile::tempdir().expect("data dir");
    let _worker = harness.worker(data.path()).await;
    let mut session = harness.session().await;
    let snapshot = harness.blobs.snapshot();
    session
        .assign(&sandbox_id(), &snapshot, v1::DesiredState::Running)
        .await;
    session
        .grant(
            JOB,
            &sandbox_id(),
            &["sh", "-c", "echo built > artifact"],
            30,
        )
        .await;
    session.result().await;

    session
        .send(Downlink::SealRequest(v1::SealRequest {
            seal_id: "seal_00000000000000000000000001".to_owned(),
            sandbox_id: sandbox_id(),
            upload_url: "http://igloo.test/v1/seals/seal_00000000000000000000000001/layer?expires=1&signature=s"
                .to_owned(),
        }))
        .await;
    let uploaded = async {
        loop {
            if let Some(upload) = harness
                .blobs
                .uploads
                .lock()
                .expect("uploads")
                .first()
                .cloned()
            {
                return upload;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let (url, bytes) = tokio::time::timeout(Duration::from_secs(10), uploaded)
        .await
        .expect("an upload within 10 s");
    let full = cfg!(target_os = "linux") && std::env::var_os("IGLOO_TEST_OVERLAY").is_some();
    assert_eq!(
        url.query_pairs()
            .any(|(key, value)| key == "layer" && value == "full"),
        !full,
        "a copied root is sealed whole: {url}"
    );
    let names: Vec<String> = tar::Archive::new(bytes.as_slice())
        .entries()
        .expect("entries")
        .map(|entry| {
            entry
                .expect("entry")
                .path()
                .expect("path")
                .display()
                .to_string()
        })
        .collect();
    assert!(
        names.iter().any(|name| name == "workspace/artifact"),
        "{names:?}"
    );
    let cached = blake3::hash(&bytes).to_hex().to_string();
    let path = data.path().join("layers").join(cached);
    let cached = async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), cached)
        .await
        .expect("the sealed layer is cached for forks");

    session
        .send(Downlink::SealRequest(v1::SealRequest {
            seal_id: "seal_00000000000000000000000002".to_owned(),
            sandbox_id: Id::<Sandbox>::from_uuid(Uuid::from_u128(77)).to_string(),
            upload_url: "http://igloo.test/v1/seals/x/layer".to_owned(),
        }))
        .await;
    let failed = session
        .until(|message| match message {
            Uplink::SealFailed(failed) => Some(failed),
            _ => None,
        })
        .await;
    assert_eq!(failed.seal_id, "seal_00000000000000000000000002");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn heartbeats_renew_a_short_lease_within_its_ttl() {
    let mut harness = Harness::start().await;
    let data = tempfile::tempdir().expect("data dir");
    let _worker = harness.worker(data.path()).await;
    let mut session = harness.session().await;
    let snapshot = harness.blobs.snapshot();
    session
        .assign(&sandbox_id(), &snapshot, v1::DesiredState::Running)
        .await;
    session
        .send(Downlink::LeaseGrant(v1::LeaseGrant {
            job_id: JOB.to_owned(),
            token: 1,
            ttl_seconds: 1,
            sandbox_id: sandbox_id(),
            argv: vec!["sleep".to_owned(), "5".to_owned()],
            env: HashMap::new(),
            timeout_seconds: 30,
        }))
        .await;
    let renewals = async {
        let mut renewals = 0;
        while renewals < 2 {
            let message = session
                .inbound
                .message()
                .await
                .expect("stream healthy")
                .expect("stream open")
                .message;
            if let Some(Uplink::Heartbeat(heartbeat)) = message
                && heartbeat.leases.iter().any(|lease| lease.job_id == JOB)
            {
                renewals += 1;
            }
        }
    };
    tokio::time::timeout(Duration::from_millis(1500), renewals)
        .await
        .expect("two renewals within 1.5 s of a 1 s lease");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_reports_usage_when_it_connects_and_then_periodically() {
    let mut harness = Harness::start().await;
    let data = tempfile::tempdir().expect("data dir");
    let _worker = harness
        .worker_reporting_every(data.path(), Duration::from_millis(200))
        .await;
    let mut session = harness.session().await;
    let first = session.hello.usage.expect("usage in the hello");
    assert!(first.disk_total_bytes > 0, "{first:?}");
    assert!(first.disk_free_bytes <= first.disk_total_bytes);
    assert_eq!(first.layer_cache_bytes, 0);
    assert_eq!(first.layer_cache_limit_bytes, 20 * 1024 * 1024 * 1024);
    assert_eq!(first.sandboxes, 0);

    let snapshot = harness.blobs.snapshot();
    session
        .assign(&sandbox_id(), &snapshot, v1::DesiredState::Running)
        .await;
    let held = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let message = session
                .inbound
                .message()
                .await
                .expect("stream healthy")
                .expect("stream open")
                .message;
            if let Some(Uplink::Usage(usage)) = message
                && usage.sandboxes == 1
                && usage.layer_cache_bytes > 0
            {
                return usage;
            }
        }
    })
    .await
    .expect("a usage report with the sandbox and its layer within 10 s");
    assert_eq!(held.layer_cache_limit_bytes, first.layer_cache_limit_bytes);
}

const TERMINAL: &str = "term_00000000000000000000000001";

impl Session {
    async fn open_terminal(&self, sandbox: &str, argv: &[&str], cols: u32, rows: u32) {
        self.send(Downlink::OpenTerminal(v1::OpenTerminal {
            terminal_id: TERMINAL.to_owned(),
            sandbox_id: sandbox.to_owned(),
            argv: argv.iter().map(ToString::to_string).collect(),
            env: HashMap::new(),
            size: Some(v1::TerminalSize { cols, rows }),
        }))
        .await;
    }

    async fn type_into(&self, text: &str) {
        self.send(Downlink::TerminalInput(v1::TerminalInput {
            terminal_id: TERMINAL.to_owned(),
            data: text.as_bytes().to_vec(),
        }))
        .await;
    }

    /// Collects terminal output until it contains `needle`; returns all of it so far.
    async fn output_until(&mut self, seen: &mut String, needle: &str) {
        while !seen.contains(needle) {
            match self.next().await {
                Uplink::TerminalOutput(output) => {
                    assert_eq!(output.terminal_id, TERMINAL);
                    seen.push_str(&String::from_utf8_lossy(&output.data));
                }
                Uplink::TerminalExit(exit) => panic!("the terminal ended early: {exit:?}\n{seen}"),
                _ => {}
            }
        }
    }

    async fn terminal_exit(&mut self) -> v1::TerminalExit {
        self.until(|message| match message {
            Uplink::TerminalExit(exit) => Some(exit),
            _ => None,
        })
        .await
    }
}

/// A running sandbox on a started worker.
async fn sandbox_ready(harness: &mut Harness, data: &Path) -> (CancellationToken, Session) {
    let worker = harness.worker(data).await;
    let mut session = harness.session().await;
    let snapshot = harness.blobs.snapshot();
    session
        .assign(&sandbox_id(), &snapshot, v1::DesiredState::Running)
        .await;
    assert_eq!(session.phase().await, v1::SandboxPhase::Starting);
    assert_eq!(session.phase().await, v1::SandboxPhase::Running);
    (worker, session)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shell_echoes_input_honours_resizes_and_reports_its_exit_code() {
    let mut harness = Harness::start().await;
    let data = tempfile::tempdir().expect("data dir");
    let (_worker, mut session) = sandbox_ready(&mut harness, data.path()).await;

    session.open_terminal(&sandbox_id(), &["sh"], 100, 30).await;
    let mut seen = String::new();
    session.type_into("echo hello-$((6*7))\n").await;
    session.output_until(&mut seen, "hello-42").await;
    assert!(
        seen.contains("echo hello"),
        "the terminal echoes typing: {seen}"
    );

    session.type_into("stty size; cat hello.txt\n").await;
    session.output_until(&mut seen, "30 100\r\nhi").await;

    session
        .send(Downlink::ResizeTerminal(v1::ResizeTerminal {
            terminal_id: TERMINAL.to_owned(),
            size: Some(v1::TerminalSize {
                cols: 132,
                rows: 43,
            }),
        }))
        .await;
    session.type_into("stty size\n").await;
    session.output_until(&mut seen, "43 132").await;

    session.type_into("exit 7\n").await;
    let exit = session.terminal_exit().await;
    assert_eq!(exit.terminal_id, TERMINAL);
    assert_eq!(exit.outcome, Some(v1::terminal_exit::Outcome::ExitCode(7)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn output_printed_before_the_exit_arrives_before_it() {
    let mut harness = Harness::start().await;
    let data = tempfile::tempdir().expect("data dir");
    let (_worker, mut session) = sandbox_ready(&mut harness, data.path()).await;

    session
        .open_terminal(
            &sandbox_id(),
            &["sh", "-c", "echo last-words; exit 3"],
            80,
            24,
        )
        .await;
    let mut seen = String::new();
    let exit = loop {
        match session.next().await {
            Uplink::TerminalOutput(output) => seen.push_str(&String::from_utf8_lossy(&output.data)),
            Uplink::TerminalExit(exit) => break exit,
            _ => {}
        }
    };
    assert!(seen.contains("last-words"), "{seen:?}");
    assert_eq!(exit.outcome, Some(v1::terminal_exit::Outcome::ExitCode(3)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_a_terminal_kills_its_process() {
    let mut harness = Harness::start().await;
    let data = tempfile::tempdir().expect("data dir");
    let (_worker, mut session) = sandbox_ready(&mut harness, data.path()).await;

    session
        .open_terminal(&sandbox_id(), &["sleep", "30"], 80, 24)
        .await;
    session
        .send(Downlink::CloseTerminal(v1::CloseTerminal {
            terminal_id: TERMINAL.to_owned(),
        }))
        .await;
    let exit = session.terminal_exit().await;
    assert_eq!(
        exit.outcome,
        Some(v1::terminal_exit::Outcome::Failure(
            v1::TerminalFailure::Closed.into()
        ))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_terminal_in_a_missing_sandbox_fails_at_once() {
    let mut harness = Harness::start().await;
    let data = tempfile::tempdir().expect("data dir");
    let _worker = harness.worker(data.path()).await;
    let mut session = harness.session().await;

    session.open_terminal(&sandbox_id(), &["sh"], 80, 24).await;
    let exit = session.terminal_exit().await;
    assert_eq!(
        exit.outcome,
        Some(v1::terminal_exit::Outcome::Failure(
            v1::TerminalFailure::SandboxUnavailable.into()
        ))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_terminal_runs_beside_a_job() {
    let mut harness = Harness::start().await;
    let data = tempfile::tempdir().expect("data dir");
    let (_worker, mut session) = sandbox_ready(&mut harness, data.path()).await;

    session.open_terminal(&sandbox_id(), &["sh"], 80, 24).await;
    session
        .grant(JOB, &sandbox_id(), &["cat", "hello.txt"], 30)
        .await;
    let result = session.result().await;
    assert_eq!(result.outcome, Some(v1::job_result::Outcome::ExitCode(0)));
    let mut seen = String::new();
    session.type_into("echo still-here\n").await;
    session.output_until(&mut seen, "still-here").await;
}
