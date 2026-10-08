//! Helpers shared by the end-to-end tests.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "the helpers of a test report failures by panicking"
)]
#![allow(
    dead_code,
    reason = "every test crate compiles this module and uses a subset of it"
)]

use std::ffi::OsString;
use std::time::Duration;

use serde_json::{Value, json};

use igloo::ci::{RunPhase, RunResource};
use igloo::job::JobResource;
use igloo::{Client, Error};
use igloo::{LogEvent, LogStream, OutputStream};
use igloo_control::{Config, Server};
use igloo_git::testing::Fixture;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};

/// A database for one test: `IGLOO_TEST_DATABASE_URL` when set, otherwise a fresh Postgres
/// container that lives as long as this value.
pub(crate) struct Database {
    _container: Option<ContainerAsync<Postgres>>,
    /// The connection URL.
    pub(crate) url: String,
}

/// What a job printed and how it ended.
#[derive(Default)]
pub(crate) struct Output {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) end: Option<JobResource>,
}

impl Database {
    pub(crate) async fn start() -> Self {
        if let Ok(url) = std::env::var("IGLOO_TEST_DATABASE_URL") {
            return Self {
                _container: None,
                url,
            };
        }
        let container = Postgres::default()
            .with_tag("17-alpine")
            .start()
            .await
            .expect("start postgres");
        let port = container.get_host_port_ipv4(5432).await.expect("port");
        Self {
            _container: Some(container),
            url: format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres"),
        }
    }
}

impl Output {
    /// Follows `logs` to the end, within `limit`.
    pub(crate) async fn of(logs: &mut LogStream, limit: Duration) -> Self {
        let mut output = Self::default();
        let follow = async {
            while let Some(event) = logs.next().await {
                match event.expect("log event") {
                    LogEvent::Output {
                        stream: OutputStream::Stdout,
                        data,
                    } => output.stdout.push_str(&data),
                    LogEvent::Output { data, .. } => output.stderr.push_str(&data),
                    LogEvent::End(job) => output.end = Some(*job),
                    _ => {}
                }
            }
        };
        tokio::time::timeout(limit, follow)
            .await
            .expect("the job finished in time");
        output
    }

    pub(crate) fn exit_code(&self) -> Option<i32> {
        self.end.as_ref().and_then(|job| job.exit_code)
    }
}

/// A bare origin repository with a work tree committing to it, compared with the API's
/// string ids.
pub(crate) struct Origin {
    _dir: tempfile::TempDir,
    fixture: Fixture,
    /// The origin's path, as registered with Igloo.
    pub(crate) path: String,
}

impl Origin {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().expect("dir");
        let fixture = Fixture::new(dir.path());
        Self {
            path: fixture.origin().display().to_string(),
            fixture,
            _dir: dir,
        }
    }

    /// Writes `files`, commits and pushes the current branch; returns the commit.
    pub(crate) fn commit(&self, files: &[(&str, &str)]) -> String {
        let files: Vec<_> = files
            .iter()
            .map(|(path, content)| (*path, Some(*content)))
            .collect();
        self.fixture.commit("change", &files).to_string()
    }

    /// Runs git in the work tree.
    pub(crate) fn git(&self, args: &[&str]) -> String {
        self.fixture.git(args)
    }

    /// The commit `branch` points at in the origin.
    pub(crate) fn head(&self, branch: &str) -> String {
        self.fixture.head(branch).to_string()
    }

    /// Creates and switches to `branch`.
    pub(crate) fn branch(&self, branch: &str) {
        self.fixture.git(&["checkout", "--quiet", "-b", branch]);
    }
}

pub(crate) const DEV_TOKEN: &str = "dev-token";

/// A server on a fresh Postgres with an embedded worker, and a client for it.
pub(crate) struct EndToEnd {
    _database: Database,
    _data: tempfile::TempDir,
    pub(crate) server: Server,
    pub(crate) client: Client,
}

impl EndToEnd {
    /// Everything job `job` printed, once it ended.
    pub(crate) async fn logs(&self, job: &str) -> String {
        let mut logs = self.client.logs(job).await.expect("logs");
        let output = Output::of(&mut logs, Duration::from_secs(60)).await;
        format!("{}{}", output.stdout, output.stderr)
    }

    pub(crate) async fn stop(self) {
        self.server
            .shutdown(Duration::from_secs(10))
            .await
            .expect("shutdown");
    }

    pub(crate) async fn start() -> Self {
        let database = Database::start().await;
        let data = tempfile::tempdir().expect("data dir");
        let config = Config::from_vars([
            ("IGLOO_LISTEN", OsString::from("127.0.0.1:0")),
            ("IGLOO_GATEWAY_LISTEN", "127.0.0.1:0".into()),
            ("IGLOO_DATABASE_URL", database.url.clone().into()),
            ("IGLOO_DATA_DIR", data.path().into()),
            ("IGLOO_DEV_TOKEN", DEV_TOKEN.into()),
            ("IGLOO_JOIN_TOKEN", "join-token".into()),
            (
                "IGLOO_SECRETS_KEY",
                "end-to-end-secrets-key-0123456789abcd".into(),
            ),
            (
                "IGLOO_BLOB_KEY",
                "end-to-end-blob-signing-secret-0123".into(),
            ),
            ("IGLOO_EMBEDDED_WORKER", "true".into()),
        ])
        .expect("config");
        let server = Server::start(&config).await.expect("server");
        let client =
            Client::new(&format!("http://{}", server.rest_address()), DEV_TOKEN).expect("client");
        Self {
            _database: database,
            _data: data,
            server,
            client,
        }
    }
}

/// The run of `change`'s revision `revision`, once ended, within two minutes.
pub(crate) async fn ended_run(client: &Client, change: &str, revision: u32) -> RunResource {
    for _ in 0..1200 {
        let runs = client.list_runs(change).await.expect("runs");
        if let Some(run) = runs.into_iter().find(|run| run.revision == revision)
            && matches!(
                run.phase,
                RunPhase::Passed | RunPhase::Failed | RunPhase::Errored
            )
        {
            return run;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the run of revision {revision} did not end");
}
/// The problem code refusing to merge `change`.
pub(crate) async fn refusal(client: &Client, change: &str) -> String {
    match client.merge_change(change).await {
        Err(Error::Api(problem)) => problem.code,
        other => panic!("the merge was not refused: {other:?}"),
    }
}

/// One MCP session over streamable HTTP, as an editor's agent opens it.
pub(crate) struct McpSession {
    http: reqwest::Client,
    url: String,
    token: &'static str,
    id: Option<String>,
    next: u64,
}

impl McpSession {
    pub(crate) async fn open(url: String, token: &'static str) -> Self {
        let mut session = Self {
            http: reqwest::Client::builder().build().expect("http"),
            url,
            token,
            id: None,
            next: 0,
        };
        let initialized = session
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": "test", "version": "1" },
                }),
            )
            .await;
        assert_eq!(initialized["serverInfo"]["name"], "igloo");
        session
            .send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await;
        session
    }

    /// Calls `method` and returns its result.
    pub(crate) async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        let body = self
            .send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        let message = body
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
            .chain(serde_json::from_str::<Value>(&body).ok())
            .find(|message| message["id"] == id)
            .unwrap_or_else(|| panic!("no response to {method}: {body}"));
        message["result"].clone()
    }

    /// Calls tool `name` and returns its structured result, asserting it succeeded.
    pub(crate) async fn call(&mut self, name: &str, arguments: Value) -> Value {
        let result = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await;
        assert_ne!(result["isError"], true, "{name}: {result}");
        result["structuredContent"].clone()
    }

    async fn send(&mut self, message: Value) -> String {
        let mut request = self
            .http
            .post(&self.url)
            .bearer_auth(self.token)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(message.to_string());
        if let Some(id) = &self.id {
            request = request.header("mcp-session-id", id);
        }
        let response = request.send().await.expect("send");
        assert!(response.status().is_success(), "{}", response.status());
        if let Some(id) = response.headers().get("mcp-session-id") {
            self.id = Some(id.to_str().expect("session id").to_owned());
        }
        response.text().await.expect("body")
    }
}
