//! The REST API end to end over memory adapters.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use igloo_core::{Actor, Digest, Id};
use jiff::SignedDuration;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::{DevToken, RestApi};
use crate::adapters::memory::FixedClock;
use crate::app::ControllerSettings;
use crate::platform::{
    ChangeModule, JobModule, RepoModule, SandboxModule, SealModule, SnapshotModule, WorkerModule,
};
use crate::testing::{FIXTURE_IMAGE, MemoryPlatform, START, blob_urls};

const TOKEN: &str = "dev-token";

fn api() -> Router {
    let MemoryPlatform { mut builder, .. } = MemoryPlatform::new();
    builder
        .install(SandboxModule {
            settings: ControllerSettings::default(),
        })
        .expect("sandbox module");
    builder
        .install(WorkerModule {
            settings: ControllerSettings::default(),
        })
        .expect("worker module");
    builder
        .install(JobModule {
            settings: ControllerSettings::default(),
        })
        .expect("job module");
    builder.install(SnapshotModule).expect("snapshot module");
    builder.install(SealModule).expect("seal module");
    builder.install(RepoModule).expect("repo module");
    builder.install(ChangeModule).expect("change module");
    let actor = Actor::Human {
        user: Id::from_uuid(Uuid::from_u128(7)),
    };
    let urls = blob_urls(Arc::clone(&builder.ports().clock));
    let api =
        RestApi::new(&builder, builder.bus(), DevToken::new(TOKEN, actor), urls).expect("api");
    let _platform = builder.build();
    api.router()
}

struct Call {
    method: Method,
    uri: String,
    body: Body,
    content_type: &'static str,
    auth: bool,
    headers: Vec<(&'static str, String)>,
}

impl Call {
    fn new(method: Method, uri: impl Into<String>) -> Self {
        Self {
            method,
            uri: uri.into(),
            body: Body::empty(),
            content_type: "application/json",
            auth: true,
            headers: Vec::new(),
        }
    }

    fn json(mut self, body: &Value) -> Self {
        self.body = Body::from(body.to_string());
        self
    }

    fn bytes(mut self, bytes: &'static [u8]) -> Self {
        self.body = Body::from(bytes);
        self.content_type = "application/octet-stream";
        self
    }

    fn raw(mut self, body: &'static str) -> Self {
        self.body = Body::from(body);
        self
    }

    fn anonymous(mut self) -> Self {
        self.auth = false;
        self
    }

    fn header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.push((name, value.to_owned()));
        self
    }

    async fn send(self, router: &Router) -> (StatusCode, Value, Vec<u8>) {
        let mut request = Request::builder()
            .method(self.method)
            .uri(self.uri)
            .header(header::CONTENT_TYPE, self.content_type);
        if self.auth {
            request = request.header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
        }
        for (name, value) in self.headers {
            request = request.header(name, value);
        }
        let response = router
            .clone()
            .oneshot(request.body(self.body).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body")
            .to_vec();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json, bytes)
    }
}

const LAYER: &[u8] = b"working tree";

fn layer_digest() -> String {
    Digest::from_blake3(*blake3::hash(LAYER).as_bytes()).to_string()
}

/// Uploads a layer, registers it as a snapshot and returns the snapshot id.
async fn snapshot(router: &Router) -> String {
    let (status, _, _) = Call::new(Method::PUT, format!("/v1/blobs/{}", layer_digest()))
        .bytes(LAYER)
        .send(router)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body, _) = Call::new(Method::POST, "/v1/snapshots")
        .json(&json!({ "layers": [{ "digest": layer_digest() }] }))
        .send(router)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().expect("id").to_owned()
}

async fn create_sandbox(router: &Router, snapshot: &str, labels: &Value) -> String {
    let (status, body, _) = Call::new(Method::POST, "/v1/sandboxes")
        .json(&json!({ "snapshot": snapshot, "labels": labels }))
        .send(router)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().expect("id").to_owned()
}

#[tokio::test]
async fn requests_without_the_token_are_unauthenticated() {
    let router = api();
    let (status, body, _) = Call::new(Method::GET, "/v1/sandboxes")
        .anonymous()
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "auth.unauthenticated");
}

#[tokio::test]
async fn blobs_and_snapshots_round_trip() {
    let router = api();
    let id = snapshot(&router).await;
    let (status, body, _) = Call::new(Method::GET, format!("/v1/snapshots/{id}"))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["layers"],
        json!([{ "digest": layer_digest(), "media_type": "tar" }])
    );
    let (status, _, bytes) = Call::new(Method::GET, format!("/v1/blobs/{}", layer_digest()))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, LAYER);
}

#[tokio::test]
async fn a_snapshot_extends_its_base() {
    let router = api();
    let base = snapshot(&router).await;
    let (status, body, _) = Call::new(Method::POST, "/v1/snapshots")
        .json(&json!({
            "base": base,
            "layers": [{ "digest": layer_digest(), "media_type": "tar" }],
        }))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["layers"].as_array().map(Vec::len), Some(2));

    let (status, body, _) = Call::new(Method::POST, "/v1/snapshots")
        .json(&json!({ "base": "nope", "layers": [{ "digest": "nope" }] }))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let fields: Vec<_> = body["errors"]
        .as_array()
        .expect("errors")
        .iter()
        .map(|error| error["field"].clone())
        .collect();
    assert_eq!(fields, [json!("base"), json!("layers.0.digest")]);
}

#[tokio::test]
async fn images_are_imported_as_snapshots() {
    let router = api();
    let import = |image: &str| {
        Call::new(Method::POST, "/v1/snapshots/import")
            .json(&json!({ "image": image, "platform": "linux/amd64" }))
    };
    let (status, first, _) = import(FIXTURE_IMAGE).send(&router).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(first["layers"][0]["media_type"], "tar+gzip");
    assert_eq!(first["layers"][1]["media_type"], "tar");
    let (status, second, _) = import(FIXTURE_IMAGE).send(&router).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(first["id"], second["id"]);

    let (status, body, _) = import("registry.test/igloo/missing:1").send(&router).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "image.not_found");
    let (status, body, _) = Call::new(Method::POST, "/v1/snapshots/import")
        .json(&json!({ "image": FIXTURE_IMAGE, "platform": "linux/s390x" }))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["errors"][0]["field"], "platform");
}

/// The path and query of `url`, as the router sees them.
fn relative(url: &reqwest::Url) -> String {
    format!("{}?{}", url.path(), url.query().expect("query"))
}

#[tokio::test]
async fn presigned_urls_need_no_token_and_open_only_their_blob() {
    let router = api();
    let digest: Digest = layer_digest().parse().expect("digest");
    let urls = blob_urls(Arc::new(FixedClock::new(START)));

    let (status, body, _) = Call::new(Method::PUT, relative(&urls.upload(digest)))
        .bytes(LAYER)
        .anonymous()
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let download = urls.download(digest);
    let (status, _, bytes) = Call::new(Method::GET, relative(&download))
        .anonymous()
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, LAYER);

    let other = Digest::from_blake3([4; 32]);
    let refused = [
        relative(&urls.upload(digest)),
        relative(&download).replace(&digest.to_string(), &other.to_string()),
        relative(&download).replace("expires=", "expires=9"),
    ];
    for uri in refused {
        let (status, body, _) = Call::new(Method::GET, uri).anonymous().send(&router).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "blob.url_invalid");
    }

    let past = blob_urls(Arc::new(FixedClock::new(
        START.saturating_add(SignedDuration::from_hours(-1)),
    )));
    let (status, body, _) = Call::new(Method::GET, relative(&past.download(digest)))
        .anonymous()
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "blob.url_expired");
}

#[tokio::test]
async fn a_blob_must_match_its_digest() {
    let router = api();
    let wrong = Digest::from_blake3([9; 32]);
    let (status, body, _) = Call::new(Method::PUT, format!("/v1/blobs/{wrong}"))
        .bytes(LAYER)
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["errors"][0]["field"], "digest");
}

#[tokio::test]
async fn sandboxes_are_created_read_stopped_and_run_jobs() {
    let router = api();
    let snapshot = snapshot(&router).await;
    let id = create_sandbox(&router, &snapshot, &json!({ "session": "42" })).await;

    let (status, body, _) = Call::new(Method::GET, format!("/v1/sandboxes/{id}"))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["phase"], "pending");
    assert_eq!(body["network"], "deny_all");

    let (status, body, _) = Call::new(Method::POST, format!("/v1/sandboxes/{id}/exec"))
        .json(&json!({ "argv": ["cargo", "test"] }))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let job = body["id"].as_str().expect("job id").to_owned();
    let (status, body, _) = Call::new(Method::GET, format!("/v1/jobs/{job}"))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["phase"], "queued");

    for _ in 0..2 {
        let (status, body, _) = Call::new(Method::POST, format!("/v1/sandboxes/{id}/stop"))
            .send(&router)
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "stop is idempotent");
        assert_eq!(body["desired"], "stopped");
    }
    let (status, body, _) = Call::new(Method::POST, format!("/v1/sandboxes/{id}/exec"))
        .json(&json!({ "argv": ["true"] }))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "job.sandbox_ended");
}

#[tokio::test]
async fn unknown_resources_are_not_found() {
    let router = api();
    for uri in [
        "/v1/sandboxes/sbx_00000000000000000000000000".to_owned(),
        "/v1/sandboxes/not-an-id".to_owned(),
        "/v1/jobs/job_00000000000000000000000000".to_owned(),
        format!("/v1/snapshots/{}", Digest::from_blake3([4; 32])),
        format!("/v1/blobs/{}", Digest::from_blake3([4; 32])),
    ] {
        let (status, body, _) = Call::new(Method::GET, uri.clone()).send(&router).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert!(
            body["code"].as_str().expect("code").ends_with(".not_found"),
            "{uri}"
        );
    }
    let (status, body, _) = Call::new(
        Method::POST,
        "/v1/sandboxes/sbx_00000000000000000000000000/stop",
    )
    .send(&router)
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "sandbox.not_found");
}

#[tokio::test]
async fn invalid_requests_report_every_field() {
    let router = api();
    let (status, body, _) = Call::new(Method::POST, "/v1/sandboxes")
        .json(&json!({
            "snapshot": "not-a-digest",
            "limits": { "millicpus": 1, "memory_mib": 2048 },
            "labels": { "Bad Key": "x" },
        }))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    insta::assert_json_snapshot!(body);
}

#[tokio::test]
async fn a_retried_create_returns_the_original_resource() {
    let router = api();
    let snapshot = snapshot(&router).await;
    let create = |labels: Value| {
        Call::new(Method::POST, "/v1/sandboxes")
            .header("idempotency-key", "create-1")
            .json(&json!({ "snapshot": snapshot, "labels": labels }))
    };
    let (status, first, _) = create(json!({ "try": "1" })).send(&router).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let (status, again, _) = create(json!({ "try": "1" })).send(&router).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(again, first, "the original response is replayed");
    let (status, list, _) = Call::new(Method::GET, "/v1/sandboxes").send(&router).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["items"].as_array().map(Vec::len), Some(1), "{list}");

    let (status, body, _) = create(json!({ "try": "2" })).send(&router).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "idempotency.key_reused");
}

#[tokio::test]
async fn a_failed_keyed_request_can_be_retried_with_its_key() {
    let router = api();
    let (status, _, _) = Call::new(Method::POST, "/v1/snapshots")
        .header("idempotency-key", "snapshot-1")
        .json(&json!({ "layers": [{ "digest": layer_digest() }] }))
        .send(&router)
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the layer is not uploaded yet"
    );
    let (status, _, _) = Call::new(Method::PUT, format!("/v1/blobs/{}", layer_digest()))
        .bytes(LAYER)
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body, _) = Call::new(Method::POST, "/v1/snapshots")
        .header("idempotency-key", "snapshot-1")
        .json(&json!({ "layers": [{ "digest": layer_digest() }] }))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[tokio::test]
async fn repositories_are_registered_once_and_their_secret_values_never_returned() {
    let router = api();
    let register = || {
        Call::new(Method::POST, "/v1/repos").json(&json!({
            "location": "github.com/roushou/igloo",
            "token_secret": "GITHUB_TOKEN",
        }))
    };
    let (status, repo, _) = register().send(&router).await;
    assert_eq!(status, StatusCode::CREATED, "{repo}");
    assert_eq!(repo["location"], "github.com/roushou/igloo");
    assert_eq!(repo["default_branch"], "main");
    let (status, body, _) = register().send(&router).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "repo.already_registered");
    let id = repo["id"].as_str().expect("id");
    let (status, list, _) = Call::new(Method::GET, "/v1/repos").send(&router).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().map(Vec::len), Some(1));

    let secret = format!("/v1/repos/{id}/secrets/GITHUB_TOKEN");
    let (status, _, _) = Call::new(Method::PUT, &secret)
        .raw("ghp_value")
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, names, raw) = Call::new(Method::GET, format!("/v1/repos/{id}/secrets"))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(names["names"], json!(["GITHUB_TOKEN"]));
    assert!(!String::from_utf8_lossy(&raw).contains("ghp_value"));
    let (status, body, _) = Call::new(Method::PUT, format!("/v1/repos/{id}/secrets/lower"))
        .raw("x")
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

    let (status, _, _) = Call::new(Method::DELETE, &secret).send(&router).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body, _) = Call::new(Method::DELETE, &secret).send(&router).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "secret.not_found");
}

#[tokio::test]
async fn a_change_records_a_revision_per_new_head_of_its_branch() {
    let router = api();
    let dir = tempfile::tempdir().expect("dir");
    let origin = igloo_git::testing::Fixture::new(dir.path());
    let fork = origin.commit("change", &[("README.md", Some("igloo"))]);
    origin.switch("feature");
    let first = origin.commit("change", &[("src/lib.rs", Some("v1"))]);
    let (status, repo, _) = Call::new(Method::POST, "/v1/repos")
        .json(&json!({ "location": origin.location().to_string() }))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{repo}");
    let changes = format!("/v1/repos/{}/changes", repo["id"].as_str().expect("id"));
    let open = || {
        Call::new(Method::POST, &changes)
            .json(&json!({ "branch": "feature", "title": "Add the library" }))
    };
    let (status, change, _) = open().send(&router).await;
    assert_eq!(status, StatusCode::CREATED, "{change}");
    assert_eq!(change["source_branch"], "feature");
    assert_eq!(change["target_branch"], "main");
    assert_eq!(change["revisions"][0]["head"], first.to_string());
    assert_eq!(change["revisions"][0]["base"], fork.to_string());
    let (status, body, _) = open().send(&router).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "change.already_open");

    let id = change["id"].as_str().expect("id");
    let revise = || Call::new(Method::POST, format!("/v1/changes/{id}/revisions"));
    let (_, unchanged, _) = revise().send(&router).await;
    assert_eq!(unchanged["revisions"].as_array().map(Vec::len), Some(1));
    let second = origin.commit("change", &[("src/lib.rs", Some("v2"))]);
    let (status, revised, _) = revise().send(&router).await;
    assert_eq!(status, StatusCode::OK, "{revised}");
    assert_eq!(revised["revisions"][1]["number"], 2);
    assert_eq!(revised["revisions"][1]["head"], second.to_string());

    let (status, closed, _) = Call::new(Method::POST, format!("/v1/changes/{id}/close"))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(closed["phase"], "closed");
    let (status, body, _) = revise().send(&router).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "change.ended");
    let (status, _, _) = open().send(&router).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "a closed change frees its branch"
    );
}

#[tokio::test]
async fn malformed_bodies_and_headers_are_rejected() {
    let router = api();
    let (status, body, _) = Call::new(Method::POST, "/v1/sandboxes")
        .raw("{")
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "request.malformed");
    let (status, body, _) = Call::new(Method::GET, "/v1/sandboxes")
        .header("idempotency-key", "has spaces")
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["errors"][0]["field"], "idempotency-key");
}

#[tokio::test]
async fn sandboxes_are_listed_in_pages_and_filtered_by_label() {
    let router = api();
    let snapshot = snapshot(&router).await;
    let mut ids = Vec::new();
    for session in ["1", "1", "2"] {
        ids.push(create_sandbox(&router, &snapshot, &json!({ "session": session })).await);
    }
    let (_, first, _) = Call::new(Method::GET, "/v1/sandboxes?limit=2")
        .send(&router)
        .await;
    assert_eq!(first["items"].as_array().expect("items").len(), 2);
    let cursor = first["next_cursor"].as_str().expect("more pages");
    let (_, second, _) = Call::new(
        Method::GET,
        format!("/v1/sandboxes?limit=2&cursor={cursor}"),
    )
    .send(&router)
    .await;
    assert_eq!(second["items"][0]["id"], ids[2]);
    assert!(second.get("next_cursor").is_none());
    let (_, filtered, _) = Call::new(Method::GET, "/v1/sandboxes?label=session=1")
        .send(&router)
        .await;
    assert_eq!(filtered["items"].as_array().expect("items").len(), 2);
    let (status, _, _) = Call::new(Method::GET, "/v1/sandboxes?limit=0")
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

/// `schemas/openapi.json` must match the served routes. `UPDATE_SCHEMAS=1` rewrites it.
#[test]
fn the_committed_openapi_document_is_current() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/openapi.json");
    let generated = RestApi::openapi().to_pretty_json().expect("serialize") + "\n";
    if std::env::var_os("UPDATE_SCHEMAS").is_some() {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("schemas directory");
        std::fs::write(&path, &generated).expect("write openapi.json");
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "schemas/openapi.json is stale; run UPDATE_SCHEMAS=1 cargo nextest run -p igloo-control"
    );
}

/// One parsed server-sent event.
#[derive(Debug)]
struct Frame {
    id: String,
    event: String,
    data: Value,
}

/// A server-sent event stream read frame by frame.
struct Sse {
    body: axum::body::BodyDataStream,
    buffer: String,
}

impl Sse {
    async fn open(router: &Router, uri: &str, headers: &[(&'static str, &str)]) -> Self {
        let mut request = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = router
            .clone()
            .oneshot(request.body(Body::empty()).expect("request"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        Self {
            body: response.into_body().into_data_stream(),
            buffer: String::new(),
        }
    }

    /// The next event, or `None` when nothing arrives within `wait`.
    async fn next(&mut self, wait: std::time::Duration) -> Option<Frame> {
        use futures_util::StreamExt;
        loop {
            if let Some((block, rest)) = self.buffer.split_once("\n\n") {
                let block = block.to_owned();
                self.buffer = rest.to_owned();
                let field = |name: &str| {
                    block
                        .lines()
                        .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
                        .map(|value| value.trim_start().to_owned())
                };
                if let (Some(id), Some(event), Some(data)) =
                    (field("id"), field("event"), field("data"))
                {
                    let data = serde_json::from_str(&data).expect("json data");
                    return Some(Frame { id, event, data });
                }
                continue;
            }
            let chunk = tokio::time::timeout(wait, self.body.next()).await.ok()??;
            self.buffer
                .push_str(&String::from_utf8_lossy(&chunk.expect("chunk")));
        }
    }

    /// Every event that arrives until the stream is quiet.
    async fn drain(&mut self) -> Vec<Frame> {
        let mut frames = Vec::new();
        while let Some(frame) = self.next(std::time::Duration::from_millis(200)).await {
            frames.push(frame);
        }
        frames
    }
}

async fn register(router: &Router, location: &str) -> String {
    let (status, repo, _) = Call::new(Method::POST, "/v1/repos")
        .json(&json!({ "location": location }))
        .send(router)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{repo}");
    repo["id"].as_str().expect("id").to_owned()
}

#[tokio::test]
async fn the_event_stream_resumes_after_the_last_event_in_order_and_once() {
    let router = api();
    register(&router, "github.com/roushou/igloo").await;
    let snapshot = snapshot(&router).await;
    create_sandbox(&router, &snapshot, &json!({})).await;
    register(&router, "github.com/roushou/other").await;

    let all = Sse::open(&router, "/v1/events?after=0", &[])
        .await
        .drain()
        .await;
    let ids: Vec<u64> = all.iter().map(|f| f.id.parse().expect("id")).collect();
    assert_eq!(ids.len(), 3, "{all:?}");
    assert!(ids.is_sorted_by(|a, b| a < b), "{ids:?}");
    assert!(all.iter().all(|f| f.event == f.data["kind"]));
    assert!(
        all.iter()
            .all(|f| f.data["sequence"] == f.id.parse::<u64>().expect("id"))
    );

    let cut = &all[0];
    let rest = Sse::open(&router, "/v1/events", &[("last-event-id", &cut.id)])
        .await
        .drain()
        .await;
    let expected: Vec<&str> = all[1..].iter().map(|f| f.id.as_str()).collect();
    assert_eq!(
        rest.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
        expected
    );
    let after = Sse::open(&router, &format!("/v1/events?after={}", cut.id), &[])
        .await
        .drain()
        .await;
    assert_eq!(after.len(), expected.len());
}

#[tokio::test]
async fn the_event_stream_can_be_limited_to_one_repository() {
    let router = api();
    let first = register(&router, "github.com/roushou/igloo").await;
    let snapshot = snapshot(&router).await;
    let sandbox = create_sandbox(&router, &snapshot, &json!({})).await;
    let second = register(&router, "github.com/roushou/other").await;

    let events = Sse::open(&router, &format!("/v1/events?after=0&repo={first}"), &[])
        .await
        .drain()
        .await;
    assert!(!events.is_empty());
    for frame in &events {
        assert_eq!(frame.data["repo"], first, "{frame:?}");
        assert_eq!(frame.data["resource_type"], "repo");
        assert_eq!(frame.data["resource_id"], first);
        assert!(frame.data.get("payload").is_none());
    }
    let everything = Sse::open(&router, "/v1/events?after=0", &[])
        .await
        .drain()
        .await;
    assert!(everything.iter().any(|f| f.data["repo"] == second));
    let unscoped = everything
        .iter()
        .find(|f| f.data["resource_id"] == sandbox)
        .expect("sandbox event");
    assert_eq!(unscoped.data["resource_type"], "sandbox");
    assert!(unscoped.data.get("repo").is_none());

    let (status, body, _) = Call::new(Method::GET, "/v1/events?repo=nope")
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let (status, _, _) = Call::new(Method::GET, "/v1/events?after=x")
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _, _) = Call::new(Method::GET, "/v1/events")
        .anonymous()
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_commands_event_reaches_a_connected_client() {
    let router = api();
    let mut stream = Sse::open(&router, "/v1/events?after=0", &[]).await;
    assert!(
        stream
            .next(std::time::Duration::from_millis(100))
            .await
            .is_none()
    );
    let repo = register(&router, "github.com/roushou/igloo").await;
    let frame = stream
        .next(std::time::Duration::from_secs(5))
        .await
        .expect("the event arrives");
    assert_eq!(frame.data["resource_id"], repo);
}

#[tokio::test]
async fn a_stream_without_a_cursor_starts_at_the_head() {
    let router = api();
    register(&router, "github.com/roushou/igloo").await;
    register(&router, "github.com/roushou/other").await;

    let mut stream = Sse::open(&router, "/v1/events", &[]).await;
    assert!(
        stream
            .next(std::time::Duration::from_millis(200))
            .await
            .is_none(),
        "events committed before the stream opened are not replayed"
    );
    let third = register(&router, "github.com/roushou/third").await;
    let frame = stream
        .next(std::time::Duration::from_secs(5))
        .await
        .expect("the new event arrives");
    assert_eq!(frame.data["resource_id"], third);
    assert_eq!(frame.id, "3");
    assert!(
        stream
            .next(std::time::Duration::from_millis(200))
            .await
            .is_none()
    );

    let history = Sse::open(&router, "/v1/events?after=0", &[])
        .await
        .drain()
        .await;
    assert_eq!(history.len(), 3);
}
