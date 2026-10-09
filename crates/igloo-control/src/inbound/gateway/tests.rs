//! The gateway against a real gRPC client over TCP.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request as HttpRequest, header};
use igloo_core::Resource;
use igloo_core::job::{JobFailure, JobId, JobPhase, JobSpec, JobTimeout};
use igloo_core::process::{Argv, EnvVars, OutputStream};
use igloo_core::sandbox::{SandboxId, SandboxPhase, SandboxSpec};
use igloo_core::worker::{Arch, Capabilities, Connection, Os, ProtocolVersion, RuntimeKind};
use igloo_core::{Actor, Digest, Id};
use igloo_worker_protocol::v1;
use igloo_worker_protocol::v1::connect_request::Message as Inbound;
use igloo_worker_protocol::v1::connect_response::Message as Outbound;
use igloo_worker_protocol::v1::worker_gateway_service_client::WorkerGatewayServiceClient;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Channel, Server};
use tonic::{Code, Request, Status, Streaming};
use tower::ServiceExt;
use uuid::Uuid;

use super::Gateway;
use crate::app::{AppError, CommandBus, ControllerSettings, TaskSupervisor};
use crate::inbound::rest::{DevToken, RestApi};
use crate::platform::{
    CancelJob, CreateSandbox, JobModule, RepoModule, SandboxModule, SealModule, SnapshotModule,
    SubmitJob, WorkerModule, WorkerUsages,
};
use crate::platform::{RegisterRepo, SetSecret};
use crate::testing::{
    MemoryPlatform, MemoryStores, blob_urls, context, register_snapshot, wait_until,
};

const JOIN_TOKEN: &str = "join-token";
const API_TOKEN: &str = "api-token";

struct Harness {
    stores: MemoryStores,
    bus: CommandBus,
    rest: Router,
    address: SocketAddr,
    supervisor: TaskSupervisor,
}

impl Harness {
    async fn start() -> Self {
        Self::with_lease_ttl(Gateway::DEFAULT_LEASE_TTL).await
    }

    async fn with_lease_ttl(lease_ttl: Duration) -> Self {
        let MemoryPlatform {
            mut builder,
            stores,
        } = MemoryPlatform::new();
        let settings = ControllerSettings::default();
        builder
            .install(SandboxModule { settings })
            .expect("sandbox module");
        builder
            .install(WorkerModule { settings })
            .expect("worker module");
        builder.install(JobModule { settings }).expect("job module");
        builder.install(SnapshotModule).expect("snapshot module");
        builder.install(SealModule).expect("seal module");
        builder.install(RepoModule).expect("repo module");
        let supervisor = TaskSupervisor::new();
        let urls = blob_urls(Arc::clone(&builder.ports().clock));
        let usages = WorkerUsages::new();
        let gateway = Gateway::new(
            &builder,
            builder.bus(),
            supervisor.spawner(),
            JOIN_TOKEN,
            urls.clone(),
        )
        .expect("gateway")
        .with_lease_ttl(lease_ttl)
        .with_usages(usages.clone());
        let actor = Actor::Human {
            user: Id::from_uuid(Uuid::from_u128(7)),
        };
        let rest = RestApi::new(
            &builder,
            builder.bus(),
            DevToken::new(API_TOKEN, actor),
            urls,
        )
        .expect("rest api")
        .with_usages(usages)
        .router();
        let bus = builder.build().start(&supervisor);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        supervisor.spawn("gateway", move |cancel| async move {
            Server::builder()
                .add_service(gateway.into_service())
                .serve_with_incoming_shutdown(TcpIncoming::from(listener), cancel.cancelled_owned())
                .await
                .map_err(AppError::infrastructure)
        });
        Self {
            stores,
            bus,
            rest,
            address,
            supervisor,
        }
    }

    async fn connect(&self, token: &str, hello: v1::Hello) -> Result<WorkerClient, Status> {
        let channel = Channel::from_shared(format!("http://{}", self.address))
            .expect("uri")
            .connect()
            .await
            .expect("channel");
        let (requests, outgoing) = mpsc::channel(16);
        requests
            .send(request(Inbound::Hello(hello)))
            .await
            .expect("send hello");
        let mut call = Request::new(ReceiverStream::new(outgoing));
        call.metadata_mut().insert(
            "authorization",
            format!("Bearer {token}").parse().expect("metadata"),
        );
        let responses = WorkerGatewayServiceClient::new(channel)
            .connect(call)
            .await?
            .into_inner();
        Ok(WorkerClient {
            requests,
            responses,
        })
    }

    async fn create_sandbox(&self) -> SandboxId {
        let spec = SandboxSpec::builder()
            .snapshot(register_snapshot(&self.bus).await)
            .build();
        self.bus
            .dispatch(CreateSandbox { spec }, context())
            .await
            .expect("create sandbox")
    }

    async fn submit_job(&self, sandbox: SandboxId) -> JobId {
        let spec = JobSpec::Execute {
            sandbox,
            argv: Argv::try_from(vec!["echo".to_owned(), "hello".to_owned()]).expect("argv"),
            env: EnvVars::default(),
            secrets: std::collections::BTreeSet::new(),
            timeout: JobTimeout::default(),
        };
        self.bus
            .dispatch(SubmitJob { spec }, context())
            .await
            .expect("submit")
    }

    async fn logs(&self, job: JobId) -> String {
        let request = HttpRequest::get(format!("/v1/jobs/{job}/logs"))
            .header(header::AUTHORIZATION, format!("Bearer {API_TOKEN}"))
            .body(Body::empty())
            .expect("request");
        let response = self.rest.clone().oneshot(request).await.expect("response");
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        String::from_utf8(body.to_vec()).expect("utf-8")
    }

    /// Downloads `url` without a token, as a worker does.
    async fn download(&self, url: &str) -> Vec<u8> {
        let url: reqwest::Url = url.parse().expect("url");
        let uri = format!("{}?{}", url.path(), url.query().expect("query"));
        let request = HttpRequest::get(uri).body(Body::empty()).expect("request");
        let response = self.rest.clone().oneshot(request).await.expect("response");
        assert!(response.status().is_success(), "{}", response.status());
        to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body")
            .to_vec()
    }

    /// Calls the REST API with the development token; seal uploads ignore it.
    async fn call(&self, method: &str, uri: &str, body: Vec<u8>) -> (u16, Vec<u8>) {
        let request = HttpRequest::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {API_TOKEN}"))
            .body(Body::from(body))
            .expect("request");
        let response = self.rest.clone().oneshot(request).await.expect("response");
        let status = response.status().as_u16();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        (status, body.to_vec())
    }

    async fn stop(self) {
        self.supervisor
            .shutdown(Duration::from_secs(5))
            .await
            .expect("shutdown");
    }
}

fn hello(worker_id: &str, protocol_version: u32) -> v1::Hello {
    let capabilities = Capabilities::new(
        Os::Linux,
        Arch::X86_64,
        [RuntimeKind::Process].into(),
        ProtocolVersion::V1,
    )
    .expect("one runtime");
    v1::Hello {
        worker_id: worker_id.to_owned(),
        protocol_version,
        capabilities: Some(v1::Capabilities::from(&capabilities)),
        labels: [("host".to_owned(), "test".to_owned())].into(),
        usage: None,
    }
}

fn request(message: Inbound) -> v1::ConnectRequest {
    v1::ConnectRequest {
        message: Some(message),
    }
}

/// A connected test worker.
struct WorkerClient {
    requests: mpsc::Sender<v1::ConnectRequest>,
    responses: Streaming<v1::ConnectResponse>,
}

impl WorkerClient {
    async fn send(&self, message: Inbound) {
        self.requests.send(request(message)).await.expect("send");
    }

    /// The next server message.
    async fn next(&mut self) -> Outbound {
        let message = tokio::time::timeout(Duration::from_secs(5), self.responses.message())
            .await
            .expect("a server message within 5 s")
            .expect("stream healthy")
            .expect("stream open");
        message.message.expect("message set")
    }

    async fn lease_grant(&mut self) -> v1::LeaseGrant {
        self.until(|message| match message {
            Outbound::LeaseGrant(grant) => Some(grant),
            _ => None,
        })
        .await
    }

    async fn result_ack(&mut self) -> v1::ResultAck {
        self.until(|message| match message {
            Outbound::ResultAck(ack) => Some(ack),
            _ => None,
        })
        .await
    }

    /// Runs the granted job: starts it, sends its output twice (as after a reconnect), renews
    /// the lease and reports `exit_code`.
    async fn run(&self, grant: &v1::LeaseGrant, exit_code: i32) {
        let (job_id, token) = (grant.job_id.clone(), grant.token);
        self.send(Inbound::JobStarted(v1::JobStarted {
            job_id: job_id.clone(),
            token,
        }))
        .await;
        let mut chunk = v1::LogChunk {
            job_id: job_id.clone(),
            token,
            offset: 0,
            data: b"hello\n".to_vec(),
            ..v1::LogChunk::default()
        };
        chunk.set_stream(OutputStream::Stdout.into());
        self.send(Inbound::LogChunk(chunk.clone())).await;
        self.send(Inbound::LogChunk(chunk)).await;
        let held = v1::HeldLease {
            job_id: job_id.clone(),
            token,
        };
        self.send(Inbound::Heartbeat(v1::Heartbeat { leases: vec![held] }))
            .await;
        self.report(&job_id, token, exit_code).await;
    }

    async fn report(&self, job_id: &str, token: u64, exit_code: i32) {
        self.send(Inbound::JobResult(v1::JobResult {
            job_id: job_id.to_owned(),
            token,
            outcome: Some(v1::job_result::Outcome::ExitCode(exit_code)),
        }))
        .await;
    }

    /// Every message received within `window`.
    async fn drain(&mut self, window: Duration) -> Vec<Outbound> {
        let mut received = Vec::new();
        let collect = async {
            while let Ok(Some(message)) = self.responses.message().await {
                if let Some(message) = message.message {
                    received.push(message);
                }
            }
        };
        let _ = tokio::time::timeout(window, collect).await;
        received
    }

    /// Skips messages until `pick` accepts one.
    async fn until<T>(&mut self, mut pick: impl FnMut(Outbound) -> Option<T>) -> T {
        loop {
            if let Some(found) = pick(self.next().await) {
                return found;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_runs_its_assigned_work_and_reports_back() {
    let harness = Harness::start().await;
    let mut worker = harness
        .connect(JOIN_TOKEN, hello("", 1))
        .await
        .expect("connect");
    let Outbound::Welcome(welcome) = worker.next().await else {
        panic!("the first message must be a welcome");
    };
    let worker_id = welcome.worker_id.clone();

    let sandbox = harness.create_sandbox().await;
    let assigned = worker
        .until(|message| match message {
            Outbound::Assignment(assignment) if !assignment.sandboxes.is_empty() => {
                Some(assignment)
            }
            _ => None,
        })
        .await;
    assert_eq!(assigned.sandboxes[0].sandbox_id, sandbox.to_string());
    let layers = &assigned.sandboxes[0].layers;
    assert_eq!(layers.len(), 1);
    assert_eq!(harness.download(&layers[0].url).await, b"working tree");
    let generation = igloo_core::Generation::INITIAL;
    worker
        .send(Inbound::SandboxStatus(v1::SandboxStatus::new(
            &sandbox.to_string(),
            SandboxPhase::Running,
            generation,
        )))
        .await;

    let job = harness.submit_job(sandbox).await;
    let grant = worker.lease_grant().await;
    assert_eq!(grant.job_id, job.to_string());
    assert_eq!(grant.argv, ["echo", "hello"]);
    worker.run(&grant, 0).await;
    let ack = worker.result_ack().await;
    assert!(ack.accepted, "{ack:?}");
    worker.report(&grant.job_id, grant.token + 1, 1).await;
    let stale = worker.result_ack().await;
    assert!(!stale.accepted);
    assert_eq!(stale.reason, "job.stale_lease");

    let stored = harness
        .stores
        .jobs
        .load(job)
        .await
        .expect("load")
        .expect("job");
    assert_eq!(stored.entity().phase(), JobPhase::Finished { exit_code: 0 });
    let sandboxes = harness.stores.sandboxes.clone();
    wait_until(async || {
        let sandbox = sandboxes
            .load(sandbox)
            .await
            .expect("load")
            .expect("sandbox");
        sandbox.entity().status().phase() == SandboxPhase::Running
    })
    .await;

    let logs = harness.logs(job).await;
    assert_eq!(
        logs.matches("event: stdout").count(),
        1,
        "resent chunks are stored once: {logs}"
    );
    assert!(logs.contains("data: hello"), "{logs}");
    assert!(logs.contains("event: end"), "{logs}");

    drop(worker);
    let workers = harness.stores.workers.clone();
    let id = worker_id.parse().expect("worker id");
    wait_until(async || {
        let worker = workers.load(id).await.expect("load").expect("worker");
        matches!(worker.entity().status(), Connection::Disconnected { .. })
    })
    .await;
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_job_is_cancelled_on_its_worker() {
    let harness = Harness::start().await;
    let mut worker = harness
        .connect(JOIN_TOKEN, hello("", 1))
        .await
        .expect("connect");
    let sandbox = harness.create_sandbox().await;
    let job = harness.submit_job(sandbox).await;
    let grant = worker.lease_grant().await;
    harness
        .bus
        .dispatch(CancelJob { job }, context())
        .await
        .expect("cancel");
    let cancel = worker
        .until(|message| match message {
            Outbound::CancelJob(cancel) => Some(cancel),
            _ => None,
        })
        .await;
    assert_eq!(cancel.job_id, grant.job_id);
    assert_eq!(cancel.token, grant.token);
    assert_eq!(job, grant.job_id.parse::<JobId>().expect("id"));
    harness.stop().await;
}

#[tokio::test]
async fn a_wrong_join_token_or_version_is_refused() {
    let harness = Harness::start().await;
    let refused = harness
        .connect("wrong", hello("", 1))
        .await
        .err()
        .expect("refused");
    assert_eq!(refused.code(), Code::Unauthenticated);
    let refused = harness
        .connect(JOIN_TOKEN, hello("", 2))
        .await
        .err()
        .expect("refused");
    assert_eq!(refused.code(), Code::FailedPrecondition);
    let unknown = format!("wrk_{}", "0".repeat(26));
    let refused = harness
        .connect(JOIN_TOKEN, hello(&unknown, 1))
        .await
        .err()
        .expect("refused");
    assert_eq!(refused.message(), "worker.unknown");
    assert_eq!(
        harness.stores.workers.all().await.expect("workers").len(),
        0
    );
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seal_reaches_the_worker_and_completes_with_its_upload() {
    let harness = Harness::start().await;
    let mut worker = harness
        .connect(JOIN_TOKEN, hello("", 1))
        .await
        .expect("connect");
    let sandbox = harness.create_sandbox().await;
    worker
        .until(|message| match message {
            Outbound::Assignment(assignment) if !assignment.sandboxes.is_empty() => Some(()),
            _ => None,
        })
        .await;
    worker
        .send(Inbound::SandboxStatus(v1::SandboxStatus::new(
            &sandbox.to_string(),
            SandboxPhase::Running,
            igloo_core::Generation::INITIAL,
        )))
        .await;
    let (status, body) = loop {
        let uri = format!("/v1/sandboxes/{sandbox}/snapshot");
        let (status, body) = harness.call("POST", &uri, Vec::new()).await;
        if status != 409 {
            break (status, body);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(status, 202, "{}", String::from_utf8_lossy(&body));
    let seal: serde_json::Value = serde_json::from_slice(&body).expect("seal");
    assert_eq!(seal["phase"], "pending");

    let request = worker
        .until(|message| match message {
            Outbound::SealRequest(request) => Some(request),
            _ => None,
        })
        .await;
    assert_eq!(request.seal_id, seal["id"]);
    assert_eq!(request.sandbox_id, sandbox.to_string());
    let url: reqwest::Url = request.upload_url.parse().expect("url");
    let unsigned = format!("{}?layer=full", url.path());
    let (status, _) = harness.call("PUT", &unsigned, b"x".to_vec()).await;
    assert_eq!(status, 400, "an upload needs its signature");
    let signed = format!("{}?{}", url.path(), url.query().expect("query"));
    let digest = |bytes: &[u8]| Digest::from_blake3(*blake3::hash(bytes).as_bytes());
    let wrong = format!("{signed}&digest={}", digest(b"other"));
    let (status, _) = harness.call("PUT", &wrong, b"layer".to_vec()).await;
    assert_eq!(status, 422, "the bytes must hash to the digest");
    let uri = format!("{signed}&digest={}", digest(b"layer"));
    let (status, body) = harness.call("PUT", &uri, b"layer".to_vec()).await;
    assert_eq!(status, 204, "{}", String::from_utf8_lossy(&body));

    let (status, body) = harness
        .call("GET", &format!("/v1/seals/{}", request.seal_id), Vec::new())
        .await;
    assert_eq!(status, 200);
    let sealed: serde_json::Value = serde_json::from_slice(&body).expect("seal");
    assert_eq!(sealed["phase"], "sealed", "{sealed}");
    let (status, body) = harness
        .call(
            "GET",
            &format!(
                "/v1/snapshots/{}",
                sealed["snapshot"].as_str().expect("snapshot")
            ),
            Vec::new(),
        )
        .await;
    assert_eq!(status, 200);
    let snapshot: serde_json::Value = serde_json::from_slice(&body).expect("snapshot");
    assert_eq!(snapshot["layers"].as_array().map(Vec::len), Some(2));
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lost_lease_fails_the_job_and_refuses_its_late_result() {
    let harness = Harness::with_lease_ttl(Duration::from_secs(1)).await;
    let mut first = harness
        .connect(JOIN_TOKEN, hello("", 1))
        .await
        .expect("connect");
    let Outbound::Welcome(welcome) = first.next().await else {
        panic!("the first message must be a welcome");
    };
    let mut second = harness
        .connect(JOIN_TOKEN, hello("", 1))
        .await
        .expect("connect a second worker");
    let sandbox = harness.create_sandbox().await;
    first
        .until(|message| match message {
            Outbound::Assignment(assignment) if !assignment.sandboxes.is_empty() => Some(()),
            _ => None,
        })
        .await;
    first
        .send(Inbound::SandboxStatus(v1::SandboxStatus::new(
            &sandbox.to_string(),
            SandboxPhase::Running,
            igloo_core::Generation::INITIAL,
        )))
        .await;
    let job = harness.submit_job(sandbox).await;
    let grant = first.lease_grant().await;
    first
        .send(Inbound::JobStarted(v1::JobStarted {
            job_id: grant.job_id.clone(),
            token: grant.token,
        }))
        .await;
    drop(first);

    let jobs = harness.stores.jobs.clone();
    wait_until(async || {
        let job = jobs.load(job).await.expect("load").expect("job");
        job.entity().phase()
            == JobPhase::Failed {
                reason: JobFailure::LeaseLost,
            }
    })
    .await;

    let mut again = harness
        .connect(JOIN_TOKEN, hello(&welcome.worker_id, 1))
        .await
        .expect("reconnect");
    again.report(&grant.job_id, grant.token, 0).await;
    let ack = again.result_ack().await;
    assert!(
        !ack.accepted,
        "a result after the lease was lost is refused"
    );
    assert_eq!(ack.reason, "job.invalid_transition");
    for worker in [&mut again, &mut second] {
        let leased = worker
            .drain(Duration::from_millis(500))
            .await
            .into_iter()
            .any(|message| matches!(message, Outbound::LeaseGrant(_)));
        assert!(!leased, "the job is never leased again");
    }
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_job_gets_its_secrets_and_their_values_are_masked_in_its_output() {
    let harness = Harness::start().await;
    let register = RegisterRepo {
        location: igloo_core::repo::RepoLocation::Local {
            path: "/srv/git/demo.git".to_owned(),
        },
        default_branch: "main".parse().expect("branch"),
        token: None,
    };
    let repo = harness
        .bus
        .dispatch(register, context())
        .await
        .expect("repo");
    let set = SetSecret {
        repo,
        name: "API_TOKEN".parse().expect("name"),
        value: crate::ports::SecretValue::try_from("hunter2".to_owned()).expect("value"),
    };
    harness.bus.dispatch(set, context()).await.expect("secret");

    let mut worker = harness
        .connect(JOIN_TOKEN, hello("", 1))
        .await
        .expect("connect");
    let spec = SandboxSpec::builder()
        .snapshot(register_snapshot(&harness.bus).await)
        .repo(repo)
        .build();
    let sandbox = harness
        .bus
        .dispatch(CreateSandbox { spec }, context())
        .await
        .expect("sandbox");
    worker
        .until(|message| match message {
            Outbound::Assignment(assignment) if !assignment.sandboxes.is_empty() => Some(()),
            _ => None,
        })
        .await;
    worker
        .send(Inbound::SandboxStatus(v1::SandboxStatus::new(
            &sandbox.to_string(),
            SandboxPhase::Running,
            igloo_core::Generation::INITIAL,
        )))
        .await;
    let spec = JobSpec::Execute {
        sandbox,
        argv: Argv::try_from(vec!["env".to_owned()]).expect("argv"),
        env: EnvVars::default(),
        secrets: ["API_TOKEN".parse().expect("name")].into(),
        timeout: JobTimeout::default(),
    };
    let job = harness
        .bus
        .dispatch(SubmitJob { spec }, context())
        .await
        .expect("submit");
    let grant = worker.lease_grant().await;
    assert_eq!(
        grant.env.get("API_TOKEN").map(String::as_str),
        Some("hunter2")
    );

    let mut chunk = v1::LogChunk {
        job_id: grant.job_id.clone(),
        token: grant.token,
        offset: 0,
        data: b"API_TOKEN=hunter2\n".to_vec(),
        ..v1::LogChunk::default()
    };
    chunk.set_stream(OutputStream::Stdout.into());
    worker.send(Inbound::LogChunk(chunk)).await;
    worker.report(&grant.job_id, grant.token, 0).await;
    worker.result_ack().await;
    let logs = harness.logs(job).await;
    assert!(logs.contains("API_TOKEN=***"), "{logs}");
    assert!(!logs.contains("hunter2"), "{logs}");
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sandbox_starts_with_its_snapshots_environment_under_its_own() {
    let harness = Harness::start().await;
    let mut worker = harness
        .connect(JOIN_TOKEN, hello("", 1))
        .await
        .expect("connect");
    let base = register_snapshot(&harness.bus).await;
    let image_env =
        EnvVars::from_pairs([("PATH", "/opt/tool/bin:/bin"), ("MODE", "image")]).expect("env");
    let snapshot = harness
        .bus
        .dispatch(
            crate::platform::RegisterSnapshot {
                base: Some(base),
                layers: Vec::new(),
                env: image_env,
            },
            context(),
        )
        .await
        .expect("snapshot");
    let spec = SandboxSpec::builder()
        .snapshot(snapshot)
        .env(EnvVars::from_pairs([("MODE", "sandbox")]).expect("env"))
        .build();
    harness
        .bus
        .dispatch(CreateSandbox { spec }, context())
        .await
        .expect("sandbox");
    let assigned = worker
        .until(|message| match message {
            Outbound::Assignment(assignment) if !assignment.sandboxes.is_empty() => {
                Some(assignment)
            }
            _ => None,
        })
        .await;
    let env = &assigned.sandboxes[0].env;
    assert_eq!(
        env.get("PATH").map(String::as_str),
        Some("/opt/tool/bin:/bin")
    );
    assert_eq!(env.get("MODE").map(String::as_str), Some("sandbox"));
    harness.stop().await;
}

fn usage(free: u64, sandboxes: u32) -> v1::Usage {
    v1::Usage {
        disk_total_bytes: 1000,
        disk_free_bytes: free,
        layer_cache_bytes: 300,
        layer_cache_limit_bytes: 500,
        sandboxes,
    }
}

impl Harness {
    /// The `usage` object of the only worker on `GET /v1/workers`.
    async fn worker_usage(&self) -> serde_json::Value {
        let (status, body) = self.call("GET", "/v1/workers", Vec::new()).await;
        assert_eq!(status, 200);
        let workers: serde_json::Value = serde_json::from_slice(&body).expect("json");
        workers[0].get("usage").cloned().unwrap_or_default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_workers_usage_appears_on_the_worker_list_and_follows_its_reports() {
    let harness = Harness::start().await;
    let mut registering = hello("", 1);
    registering.usage = Some(usage(400, 1));
    let mut client = harness
        .connect(JOIN_TOKEN, registering)
        .await
        .expect("connected");
    client.next().await;
    wait_until(|| async { !harness.worker_usage().await.is_null() }).await;
    let reported = harness.worker_usage().await;
    assert_eq!(reported["disk_total_bytes"], 1000);
    assert_eq!(reported["disk_free_bytes"], 400);
    assert_eq!(reported["layer_cache_bytes"], 300);
    assert_eq!(reported["layer_cache_limit_bytes"], 500);
    assert_eq!(reported["sandboxes"], 1);
    assert!(reported["reported_at"].is_string());

    client.send(Inbound::Usage(usage(250, 2))).await;
    wait_until(|| async { harness.worker_usage().await["disk_free_bytes"] == 250 }).await;
    assert_eq!(harness.worker_usage().await["sandboxes"], 2);
    harness.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_that_reports_no_usage_is_accepted_and_listed_without_it() {
    let harness = Harness::start().await;
    let mut client = harness
        .connect(JOIN_TOKEN, hello("", 1))
        .await
        .expect("an older worker connects");
    client.next().await;
    let (_, body) = harness.call("GET", "/v1/workers", Vec::new()).await;
    let workers: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(workers.as_array().map(Vec::len), Some(1));
    assert!(workers[0].get("usage").is_none());
    harness.stop().await;
}
