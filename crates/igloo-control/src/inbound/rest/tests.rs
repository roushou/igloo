//! The REST API end to end over memory adapters.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use igloo_core::repo::Repo;
use igloo_core::{Actor, Digest, Id};
use jiff::SignedDuration;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::{DevToken, RestApi};
use crate::adapters::memory::FixedClock;
use crate::app::ControllerSettings;
use crate::ci::{CheckOutcome, CheckSpec, Planned, Run, SandboxSettings};
use crate::platform::{
    ChangeModule, JobModule, RepoModule, SandboxModule, SealModule, SnapshotModule, WorkerModule,
};
use crate::ports::{CommitMeta, CorrelationId, EntityStore, Versioned};
use crate::testing::{FIXTURE_IMAGE, MemoryPlatform, MemoryStores, START, blob_urls, public_url};
use crate::workspaces::WorkspaceModule;

const TOKEN: &str = "dev-token";

fn api() -> Router {
    api_with_stores().0
}

/// The API, with the stores tests seed state through.
fn api_with_stores() -> (Router, Arc<dyn EntityStore<Run>>, MemoryStores) {
    let (router, runs, _repos, stores) = api_parts();
    (router, runs, stores)
}

/// The API, with every store tests seed state through.
#[allow(clippy::type_complexity, reason = "a tuple of the stores a test seeds")]
fn api_parts() -> (
    Router,
    Arc<dyn EntityStore<Run>>,
    Arc<dyn EntityStore<Repo>>,
    MemoryStores,
) {
    let MemoryPlatform {
        mut builder,
        stores,
    } = MemoryPlatform::new();
    let runs = builder.store::<Run>().expect("run store");
    let repos = builder.store::<Repo>().expect("repo store");
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
    builder
        .install(WorkspaceModule {
            settings: ControllerSettings::default(),
            git_base: public_url(),
        })
        .expect("workspace module");
    let actor = Actor::Human {
        user: Id::from_uuid(Uuid::from_u128(7)),
    };
    let urls = blob_urls(Arc::clone(&builder.ports().clock));
    let api =
        RestApi::new(&builder, builder.bus(), DevToken::new(TOKEN, actor), urls).expect("api");
    let _platform = builder.build();
    (api.router(), runs, repos, stores)
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
async fn a_repositorys_dotfiles_are_set_reported_and_cleared() {
    let router = api();
    let id = register(&router, "github.com/roushou/igloo").await;
    let path = format!("/v1/repos/{id}/dotfiles");
    let dotfiles =
        json!({ "repository": "https://github.com/me/dotfiles", "install": "./install.sh" });

    let (status, repo, _) = Call::new(Method::PUT, &path)
        .json(&dotfiles)
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::OK, "{repo}");
    assert_eq!(repo["dotfiles"], dotfiles);
    let (_, repo, _) = Call::new(Method::GET, format!("/v1/repos/{id}"))
        .send(&router)
        .await;
    assert_eq!(repo["dotfiles"], dotfiles);

    let invalid = json!({ "repository": "nope", "install": " " });
    let (status, body, _) = Call::new(Method::PUT, &path)
        .json(&invalid)
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let (_, repo, _) = Call::new(Method::GET, format!("/v1/repos/{id}"))
        .send(&router)
        .await;
    assert_eq!(
        repo["dotfiles"], dotfiles,
        "a rejected request changes nothing"
    );

    let (status, _, _) = Call::new(Method::DELETE, &path).send(&router).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, repo, _) = Call::new(Method::GET, format!("/v1/repos/{id}"))
        .send(&router)
        .await;
    assert!(repo.get("dotfiles").is_none(), "{repo}");

    let unknown = "/v1/repos/repo_00000000000000000000000000/dotfiles";
    let (status, _, _) = Call::new(Method::PUT, unknown)
        .json(&dotfiles)
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
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

/// Clients address operations by `operationId`, so each must name exactly one.
#[test]
fn every_operation_id_is_unique() {
    let document = serde_json::to_value(RestApi::openapi()).expect("serialize");
    let mut seen = std::collections::BTreeSet::new();
    for (path, item) in document["paths"].as_object().expect("paths") {
        for (method, operation) in item.as_object().expect("path item") {
            let id = operation["operationId"].as_str();
            let id = id.unwrap_or_else(|| panic!("{method} {path} has no operationId"));
            assert!(
                seen.insert(id.to_owned()),
                "{id} names more than one operation"
            );
        }
    }
}

/// Registers `origin` and returns the repository's id.
async fn register_origin(router: &Router, origin: &igloo_git::testing::Fixture) -> String {
    let (status, repo, _) = Call::new(Method::POST, "/v1/repos")
        .json(&json!({ "location": origin.location().to_string() }))
        .send(router)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{repo}");
    repo["id"].as_str().expect("id").to_owned()
}

async fn open_change(router: &Router, repo: &str, branch: &str) -> Value {
    let (status, change, _) = Call::new(Method::POST, format!("/v1/repos/{repo}/changes"))
        .json(&json!({ "branch": branch, "title": format!("Change {branch}") }))
        .send(router)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{change}");
    change
}

/// Stores a run of `revision` of `change` that ran one check and ended with `outcome`.
async fn store_run(runs: &Arc<dyn EntityStore<Run>>, change: &Value, n: u128, passed: bool) {
    let change_id = change["id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("change id");
    let repo = change["repo"]
        .as_str()
        .expect("repo")
        .parse()
        .expect("repo id");
    let commit = change["revisions"][0]["head"]
        .as_str()
        .expect("head")
        .parse()
        .expect("commit");
    let id = Id::from_uuid(Uuid::from_u128(n));
    let mut run = Run::new(id, repo, (change_id, 1), commit, START);
    let planned = Planned {
        settings: SandboxSettings {
            isolation: igloo_core::sandbox::Isolation::Any,
            limits: igloo_core::sandbox::ResourceLimits::default(),
            network: igloo_core::sandbox::NetworkPolicy::DenyAll,
            env: igloo_core::process::EnvVars::default(),
            secrets: std::collections::BTreeSet::new(),
        },
        checks: vec![CheckSpec {
            name: "test".to_owned(),
            run: "true".to_owned(),
            timeout_seconds: 60,
        }],
    };
    let snapshot = igloo_core::snapshot::SnapshotId::from(Digest::from_blake3([1; 32]));
    run.prepared(planned, Some(snapshot), None)
        .expect("prepared");
    let job = Id::from_uuid(Uuid::from_u128(n + 1000));
    run.checks_started(Id::from_uuid(Uuid::from_u128(n + 2000)), vec![job])
        .expect("started");
    let outcome = if passed {
        CheckOutcome::Passed
    } else {
        CheckOutcome::Failed { exit_code: 1 }
    };
    run.job_ended(job, outcome);
    let meta = CommitMeta {
        actor: Actor::Human {
            user: Id::from_uuid(Uuid::from_u128(7)),
        },
        time: START,
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(n)),
        causation_id: None,
    };
    let mut run = Versioned::new(run);
    runs.commit(&mut run, &meta).await.expect("commit run");
}

#[tokio::test]
async fn changes_are_filtered_by_phase_and_ordered() {
    let router = api();
    let dir = tempfile::tempdir().expect("dir");
    let origin = igloo_git::testing::Fixture::new(dir.path());
    origin.commit("base", &[("README.md", Some("igloo"))]);
    for branch in ["first", "second", "third"] {
        origin.switch(branch);
        origin.commit(branch, &[(branch, Some(branch))]);
    }
    let repo = register_origin(&router, &origin).await;
    let mut ids = Vec::new();
    for branch in ["first", "second", "third"] {
        ids.push(open_change(&router, &repo, branch).await["id"].clone());
    }
    let (status, _, _) = Call::new(
        Method::POST,
        format!("/v1/changes/{}/close", ids[0].as_str().expect("id")),
    )
    .send(&router)
    .await;
    assert_eq!(status, StatusCode::OK);

    let list = |query: &'static str| {
        let router = router.clone();
        let uri = format!("/v1/repos/{repo}/changes{query}");
        async move {
            let (status, body, _) = Call::new(Method::GET, uri).send(&router).await;
            (status, body)
        }
    };
    let ids_of = |body: &Value| -> Vec<Value> {
        body.as_array()
            .expect("array")
            .iter()
            .map(|change| change["id"].clone())
            .collect()
    };
    let (_, all) = list("").await;
    assert_eq!(ids_of(&all), ids, "oldest first by default");
    let (_, open) = list("?phase=open").await;
    assert_eq!(ids_of(&open), ids[1..], "only open changes");
    let (_, newest) = list("?order=newest").await;
    assert_eq!(
        ids_of(&newest),
        [ids[2].clone(), ids[1].clone(), ids[0].clone()]
    );
    let (_, either) = list("?phase=closed&phase=merged&order=newest").await;
    assert_eq!(ids_of(&either), [ids[0].clone()]);
    let (status, body) = list("?phase=bogus&order=sideways").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "validation.invalid");
}

#[tokio::test]
async fn merge_readiness_agrees_with_what_merging_requires() {
    let (router, runs, _stores) = api_with_stores();
    let dir = tempfile::tempdir().expect("dir");
    let origin = igloo_git::testing::Fixture::new(dir.path());
    origin.commit(
        "base",
        &[(".igloo/agents.toml", Some("protected = [\"api/**\"]\n"))],
    );
    origin.switch("feature");
    origin.commit(
        "change",
        &[("api/schema.json", Some("{}")), ("src/lib.rs", Some("v1"))],
    );
    let repo = register_origin(&router, &origin).await;
    let change = open_change(&router, &repo, "feature").await;
    let id = change["id"].as_str().expect("id");
    let get = || async {
        let (status, body, _) = Call::new(Method::GET, format!("/v1/changes/{id}"))
            .send(&router)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["readiness"].clone()
    };
    let merge = || async {
        let (status, body, _) = Call::new(Method::POST, format!("/v1/changes/{id}/merge"))
            .send(&router)
            .await;
        (status, body)
    };

    let readiness = get().await;
    assert_eq!(readiness["checks"], "missing");
    assert_eq!(readiness["approval"]["state"], "required");
    assert_eq!(
        readiness["approval"]["protected_paths"],
        json!(["api/schema.json"])
    );
    assert_eq!(readiness["fast_forward"], true);
    assert_eq!(merge().await.1["code"], "change.checks_not_passed");

    store_run(&runs, &change, 1, true).await;
    let readiness = get().await;
    assert_eq!(readiness["checks"], "passed");
    assert_eq!(
        readiness["approval"]["state"], "required",
        "checks do not approve"
    );
    let (status, body) = merge().await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "change.approval_required");

    let (status, body, _) = Call::new(Method::POST, format!("/v1/changes/{id}/approve"))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let readiness = get().await;
    assert_eq!(readiness["approval"]["state"], "given");
    assert_eq!(
        readiness["approval"]["protected_paths"],
        json!(["api/schema.json"])
    );
    let (status, body) = merge().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["phase"], "merged");
    assert!(body["merged_at"].is_string(), "{body}");
    assert!(body.get("closed_at").is_none());
    assert!(
        body.get("readiness").is_none(),
        "only open changes have readiness"
    );
}

#[tokio::test]
async fn readiness_reports_failed_checks_and_a_moved_target() {
    let (router, runs, _stores) = api_with_stores();
    let dir = tempfile::tempdir().expect("dir");
    let origin = igloo_git::testing::Fixture::new(dir.path());
    origin.commit("base", &[("README.md", Some("igloo"))]);
    origin.switch("feature");
    origin.commit("change", &[("src/lib.rs", Some("v1"))]);
    let repo = register_origin(&router, &origin).await;
    let change = open_change(&router, &repo, "feature").await;
    let id = change["id"].as_str().expect("id");
    store_run(&runs, &change, 1, false).await;
    origin.switch("main");
    origin.commit("move", &[("other.txt", Some("x"))]);
    let readiness = || async {
        let (_, body, _) = Call::new(Method::GET, format!("/v1/changes/{id}"))
            .send(&router)
            .await;
        body["readiness"].clone()
    };
    let before = readiness().await;
    assert_eq!(before["checks"], "failed");
    assert_eq!(before["approval"]["state"], "not_required");
    assert_eq!(
        before["fast_forward"], true,
        "readiness judges the mirror as last fetched"
    );
    origin.switch("other");
    origin.commit("other", &[("other", Some("y"))]);
    open_change(&router, &repo, "other").await;
    assert_eq!(readiness().await["fast_forward"], false);
}

#[tokio::test]
async fn readiness_is_answered_without_reaching_the_forge() {
    let (router, _runs, _stores) = api_with_stores();
    let dir = tempfile::tempdir().expect("dir");
    let origin = igloo_git::testing::Fixture::new(dir.path());
    origin.commit("base", &[("README.md", Some("igloo"))]);
    origin.switch("feature");
    origin.commit("change", &[("src/lib.rs", Some("v1"))]);
    let repo = register_origin(&router, &origin).await;
    let change = open_change(&router, &repo, "feature").await;
    let id = change["id"].as_str().expect("id");
    std::fs::remove_dir_all(origin.origin()).expect("remove the origin");
    for uri in [
        format!("/v1/changes/{id}"),
        format!("/v1/repos/{repo}/changes"),
    ] {
        let (status, body, _) = Call::new(Method::GET, uri).send(&router).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let change = if body.is_array() { &body[0] } else { &body };
        assert_eq!(change["readiness"]["fast_forward"], true);
    }
    let (status, _, _) = Call::new(Method::POST, format!("/v1/changes/{id}/merge"))
        .send(&router)
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "merge still needs passing checks first"
    );
}

#[tokio::test]
async fn runs_of_a_repository_are_listed_newest_first_in_pages() {
    let (router, runs, _stores) = api_with_stores();
    let dir = tempfile::tempdir().expect("dir");
    let origin = igloo_git::testing::Fixture::new(dir.path());
    origin.commit("base", &[("README.md", Some("igloo"))]);
    origin.switch("feature");
    origin.commit("change", &[("src/lib.rs", Some("v1"))]);
    let repo = register_origin(&router, &origin).await;
    let change = open_change(&router, &repo, "feature").await;
    for n in 1..=3 {
        store_run(&runs, &change, n, true).await;
    }
    let uri = format!("/v1/repos/{repo}/runs");
    let (status, page, _) = Call::new(Method::GET, format!("{uri}?limit=2"))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let ids = |page: &Value| -> Vec<String> {
        page["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|run| run["id"].as_str().expect("id").to_owned())
            .collect()
    };
    let first = ids(&page);
    assert_eq!(first.len(), 2);
    assert!(first[0] > first[1], "newest first");
    let cursor = page["next_cursor"].as_str().expect("more pages");
    let (_, rest, _) = Call::new(Method::GET, format!("{uri}?limit=2&cursor={cursor}"))
        .send(&router)
        .await;
    let last = ids(&rest);
    assert_eq!(last.len(), 1);
    assert!(last[0] < first[1]);
    assert!(rest.get("next_cursor").is_none());
    let (status, _, _) = Call::new(Method::GET, format!("{uri}?limit=0"))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _, _) = Call::new(Method::GET, "/v1/repos/repo_unknown/runs")
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn workers_are_listed_with_what_their_sandboxes_hold() {
    let (router, _runs, stores) = api_with_stores();
    let (status, workers, _) = Call::new(Method::GET, "/v1/workers").send(&router).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(workers, json!([]));

    let capabilities = igloo_core::worker::Capabilities::new(
        igloo_core::worker::Os::Linux,
        igloo_core::worker::Arch::X86_64,
        [igloo_core::worker::RuntimeKind::Process].into(),
        igloo_core::worker::ProtocolVersion::V1,
    )
    .expect("capabilities");
    let worker_id: igloo_core::worker::WorkerId = Id::from_uuid(Uuid::from_u128(50));
    let worker = igloo_core::worker::Worker::new(
        worker_id,
        capabilities,
        igloo_core::Labels::from_pairs([("zone", "a")]).expect("labels"),
    );
    let mut worker = Versioned::new(worker);
    let meta = CommitMeta {
        actor: Actor::Human {
            user: Id::from_uuid(Uuid::from_u128(7)),
        },
        time: START,
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(1)),
        causation_id: None,
    };
    stores
        .workers
        .commit(&mut worker, &meta)
        .await
        .expect("worker");

    let snapshot = snapshot(&router).await;
    let mut ids = Vec::new();
    for _ in 0..2 {
        ids.push(create_sandbox(&router, &snapshot, &json!({})).await);
    }
    for id in &ids {
        let mut sandbox = stores
            .sandboxes
            .load(id.parse().expect("sandbox id"))
            .await
            .expect("load")
            .expect("sandbox");
        sandbox
            .entity_mut()
            .schedule(igloo_core::Entity::id(worker.entity()))
            .expect("schedule");
        stores
            .sandboxes
            .commit(&mut sandbox, &meta)
            .await
            .expect("commit");
    }
    let (_, workers, _) = Call::new(Method::GET, "/v1/workers").send(&router).await;
    let worker = &workers[0];
    assert_eq!(worker["id"], worker_id.to_string());
    assert_eq!(worker["labels"], json!({ "zone": "a" }));
    assert_eq!(worker["connection"], "connected");
    assert_eq!(worker["schedulability"], "schedulable");
    assert_eq!(worker["schedulable"], true);
    assert_eq!(worker["capabilities"]["os"], "linux");
    assert_eq!(worker["capabilities"]["runtimes"], json!(["process"]));
    let default = igloo_core::sandbox::ResourceLimits::default();
    assert_eq!(worker["allocated"]["sandboxes"], 2);
    assert_eq!(
        worker["allocated"]["millicpus"],
        2 * u64::from(default.millicpus())
    );
    assert_eq!(
        worker["allocated"]["memory_mib"],
        2 * u64::from(default.memory_mib())
    );
}

#[tokio::test]
async fn a_changes_diff_lists_its_files_with_patches_per_revision() {
    let router = api();
    let dir = tempfile::tempdir().expect("dir");
    let origin = igloo_git::testing::Fixture::new(dir.path());
    let body = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n";
    let fork = origin.commit(
        "base",
        &[
            ("keep", Some("1\n2\n")),
            ("gone", Some("bye\n")),
            ("old name", Some(body)),
            ("blob", Some("a\0b")),
        ],
    );
    origin.switch("feature");
    let huge = "x\n".repeat(200_000);
    let first = origin.commit(
        "first",
        &[
            ("keep", Some("1\n2\n3\n")),
            ("gone", None),
            ("old name", None),
            (
                "new name",
                Some("one\ntwo\nthree\nfour\nfive\nsix\nseven\nEIGHT\n"),
            ),
            ("added", Some("hello\n")),
            ("blob", Some("a\0c")),
            ("huge", Some(&huge)),
        ],
    );
    let repo = register_origin(&router, &origin).await;
    let change = open_change(&router, &repo, "feature").await;
    let id = change["id"].as_str().expect("id");

    let (status, diff, _) = Call::new(Method::GET, format!("/v1/changes/{id}/diff"))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::OK, "{diff}");
    assert_eq!(diff["revision"], 1);
    assert_eq!(diff["base"], fork.to_string());
    assert_eq!(diff["head"], first.to_string());
    let files = diff["files"].as_array().expect("files");
    let summary: Vec<_> = files
        .iter()
        .map(|file| {
            (
                file["path"].as_str().expect("path"),
                file["status"].as_str().expect("status"),
                file["additions"].as_u64().expect("additions"),
                file["deletions"].as_u64().expect("deletions"),
                file["binary"].as_bool().expect("binary"),
                file["truncated"].as_bool().expect("truncated"),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("added", "added", 1, 0, false, false),
            ("blob", "modified", 0, 0, true, false),
            ("gone", "deleted", 0, 1, false, false),
            ("huge", "added", 200_000, 0, false, true),
            ("keep", "modified", 1, 0, false, false),
            ("new name", "renamed", 1, 1, false, false),
        ]
    );
    let file = |path: &str| {
        files
            .iter()
            .find(|file| file["path"] == path)
            .expect("file")
    };
    assert_eq!(file("new name")["previous_path"], "old name");
    assert!(file("added").get("previous_path").is_none());
    assert!(file("blob").get("patch").is_none());
    assert!(file("huge").get("patch").is_none(), "over 256 KiB");
    assert_eq!(
        file("keep")["patch"],
        "diff --git a/keep b/keep\nindex 1191247..01e79c3 100644\n--- a/keep\n+++ b/keep\n\
         @@ -1,2 +1,3 @@\n 1\n 2\n+3\n"
    );
}

#[tokio::test]
async fn a_changes_diff_follows_its_revisions() {
    let router = api();
    let dir = tempfile::tempdir().expect("dir");
    let origin = igloo_git::testing::Fixture::new(dir.path());
    let fork = origin.commit("base", &[("added", Some("hello\n"))]);
    origin.switch("feature");
    let first = origin.commit("first", &[("added", Some("hello\nagain\n"))]);
    let repo = register_origin(&router, &origin).await;
    let change = open_change(&router, &repo, "feature").await;
    let id = change["id"].as_str().expect("id");
    let diff_of =
        |query: &str| Call::new(Method::GET, format!("/v1/changes/{id}/diff{query}")).send(&router);

    let (_, diff, _) = diff_of("").await;
    assert_eq!(
        (diff["revision"].as_u64(), &diff["head"]),
        (Some(1), &json!(first.to_string()))
    );

    let second = origin.commit("second", &[("added", Some("hello\nagain\nmore\n"))]);
    let (_, revised, _) = Call::new(Method::POST, format!("/v1/changes/{id}/revisions"))
        .send(&router)
        .await;
    assert_eq!(revised["revisions"][1]["head"], second.to_string());
    let (_, latest, _) = diff_of("").await;
    assert_eq!(latest["revision"], 2);
    assert_eq!(latest["base"], fork.to_string());
    assert_eq!(latest["head"], second.to_string());
    assert_eq!(latest["files"][0]["additions"], 2);
    let (_, earlier, _) = diff_of("?revision=1").await;
    assert_eq!(earlier, diff, "an earlier revision keeps its own diff");

    let (status, missing, _) = diff_of("?revision=3").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(missing["code"], "change.revision_not_found");
    let unknown = "chg_00000000000000000000000000";
    let (status, missing, _) = Call::new(Method::GET, format!("/v1/changes/{unknown}/diff"))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(missing["code"], "change.not_found");
}

/// Commits `entity`'s new events to `store` as of `START + seconds`.
async fn commit_at<E>(store: &Arc<dyn EntityStore<E>>, entity: &mut Versioned<E>, seconds: i64)
where
    E: igloo_core::Entity + Send + Sync + 'static,
{
    let meta = CommitMeta {
        actor: Actor::Human {
            user: Id::from_uuid(Uuid::from_u128(7)),
        },
        time: START.saturating_add(SignedDuration::from_secs(seconds)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(u128::from(
            seconds.unsigned_abs(),
        ))),
        causation_id: None,
    };
    store.commit(entity, &meta).await.expect("commit");
}

fn at(seconds: i64) -> String {
    START
        .saturating_add(SignedDuration::from_secs(seconds))
        .to_string()
}

/// Walks job `id` through lease, start and finish, committing each step at the given second.
async fn run_job(stores: &MemoryStores, id: u128, started: i64, finished: i64) {
    let spec = igloo_api::job::ExecRequest::new(vec!["true".to_owned()])
        .into_spec(Id::from_uuid(Uuid::from_u128(id + 1)))
        .expect("spec");
    let id = Id::from_uuid(Uuid::from_u128(id));
    let mut job = Versioned::new(igloo_core::job::Job::new(id, spec, START));
    commit_at(&stores.jobs, &mut job, 0).await;
    let lease = job
        .entity_mut()
        .lease(
            Id::from_uuid(Uuid::from_u128(9)),
            SignedDuration::from_secs(60),
            START,
        )
        .expect("lease");
    job.entity_mut().start(lease.token()).expect("start");
    commit_at(&stores.jobs, &mut job, started).await;
    job.entity_mut().finish(lease.token(), 0).expect("finish");
    commit_at(&stores.jobs, &mut job, finished).await;
}

#[tokio::test]
async fn jobs_report_when_they_started_and_ended() {
    let (router, _runs, stores) = api_with_stores();
    run_job(&stores, 5000, 10, 25).await;
    let (status, job, _) = Call::new(
        Method::GET,
        format!(
            "/v1/jobs/{}",
            Id::<igloo_core::job::Job>::from_uuid(Uuid::from_u128(5000))
        ),
    )
    .send(&router)
    .await;
    assert_eq!(status, StatusCode::OK, "{job}");
    assert_eq!(job["phase"], "finished");
    assert_eq!(job["started_at"], at(10));
    assert_eq!(job["ended_at"], at(25));
}

#[tokio::test]
async fn a_queued_job_has_neither_time() {
    let router = api();
    let snapshot = snapshot(&router).await;
    let sandbox = create_sandbox(&router, &snapshot, &json!({})).await;
    let (_, job, _) = Call::new(Method::POST, format!("/v1/sandboxes/{sandbox}/exec"))
        .json(&json!({ "argv": ["true"] }))
        .send(&router)
        .await;
    assert!(job.get("started_at").is_none());
    assert!(job.get("ended_at").is_none());
}

#[tokio::test]
async fn runs_and_their_checks_report_their_times_and_sandbox() {
    let (router, runs, stores) = api_with_stores();
    let dir = tempfile::tempdir().expect("dir");
    let origin = igloo_git::testing::Fixture::new(dir.path());
    origin.commit("base", &[("README.md", Some("igloo"))]);
    origin.switch("feature");
    origin.commit("change", &[("src/lib.rs", Some("v1"))]);
    let repo = register_origin(&router, &origin).await;
    let change = open_change(&router, &repo, "feature").await;
    store_run(&runs, &change, 1, true).await;
    run_job(&stores, 1001, 4, 9).await;
    let (status, run, _) = Call::new(
        Method::GET,
        format!("/v1/runs/{}", Id::<Run>::from_uuid(Uuid::from_u128(1))),
    )
    .send(&router)
    .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    assert_eq!(run["phase"], "passed");
    assert_eq!(
        run["ended_at"],
        at(0),
        "the last check ended at the run's commit"
    );
    assert_eq!(
        run["sandbox"],
        Id::<igloo_core::sandbox::Sandbox>::from_uuid(Uuid::from_u128(2001)).to_string()
    );
    assert_eq!(run["checks"][0]["started_at"], at(4));
    assert_eq!(run["checks"][0]["ended_at"], at(9));
}

#[tokio::test]
async fn a_running_run_has_no_end() {
    let (router, runs, _stores) = api_with_stores();
    let dir = tempfile::tempdir().expect("dir");
    let origin = igloo_git::testing::Fixture::new(dir.path());
    origin.commit("base", &[("README.md", Some("igloo"))]);
    origin.switch("feature");
    origin.commit("change", &[("src/lib.rs", Some("v1"))]);
    let repo = register_origin(&router, &origin).await;
    let change = open_change(&router, &repo, "feature").await;
    let run = Run::new(
        Id::from_uuid(Uuid::from_u128(1)),
        repo.parse().expect("repo"),
        (change["id"].as_str().expect("id").parse().expect("id"), 1),
        change["revisions"][0]["head"]
            .as_str()
            .expect("head")
            .parse()
            .expect("commit"),
        START,
    );
    commit_at(&runs, &mut Versioned::new(run), 0).await;
    let (_, run, _) = Call::new(
        Method::GET,
        format!("/v1/runs/{}", Id::<Run>::from_uuid(Uuid::from_u128(1))),
    )
    .send(&router)
    .await;
    assert_eq!(run["phase"], "preparing");
    assert!(run.get("ended_at").is_none());
    assert!(run.get("sandbox").is_none());
}

#[tokio::test]
async fn changes_report_when_they_were_closed() {
    let router = api();
    let dir = tempfile::tempdir().expect("dir");
    let origin = igloo_git::testing::Fixture::new(dir.path());
    origin.commit("base", &[("README.md", Some("igloo"))]);
    origin.switch("feature");
    origin.commit("change", &[("src/lib.rs", Some("v1"))]);
    let repo = register_origin(&router, &origin).await;
    let change = open_change(&router, &repo, "feature").await;
    assert!(change.get("closed_at").is_none());
    assert!(change.get("merged_at").is_none());
    let id = change["id"].as_str().expect("id");
    let (_, closed, _) = Call::new(Method::POST, format!("/v1/changes/{id}/close"))
        .send(&router)
        .await;
    assert!(closed["closed_at"].is_string(), "{closed}");
    assert!(closed.get("merged_at").is_none());
    let (_, listed, _) = Call::new(Method::GET, format!("/v1/repos/{repo}/changes"))
        .send(&router)
        .await;
    assert_eq!(listed[0]["closed_at"], closed["closed_at"]);
}

#[tokio::test]
async fn a_repository_lists_its_recorded_snapshots() {
    let (router, _runs, repos, _stores) = api_parts();
    let snapshot = snapshot(&router).await;
    let repo_id = Id::from_uuid(Uuid::from_u128(77));
    let mut repo = Versioned::new(Repo::new(
        repo_id,
        igloo_core::repo::RepoLocation::Local {
            path: "/srv/origin".to_owned(),
        },
        "main".parse().expect("branch"),
        None,
        START,
    ));
    commit_at(&repos, &mut repo, 0).await;
    let uri = format!("/v1/repos/{repo_id}/snapshots");
    let (status, none, _) = Call::new(Method::GET, &uri).send(&router).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(none, json!([]));

    let key = Digest::from_blake3([3; 32]);
    repo.entity_mut().record_warm(
        key,
        igloo_core::repo::WarmSnapshot {
            snapshot: snapshot.parse().expect("snapshot"),
            commit: "a".repeat(40).parse().expect("commit"),
        },
    );
    commit_at(&repos, &mut repo, 30).await;
    let (status, listed, _) = Call::new(Method::GET, &uri).send(&router).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed[0]["key"], key.to_string());
    assert_eq!(listed[0]["snapshot"], snapshot);
    assert_eq!(listed[0]["commit"], "a".repeat(40));
    assert_eq!(listed[0]["built_at"], at(30));
    assert!(
        listed[0].get("last_used_at").is_none(),
        "no use is recorded yet"
    );
    assert!(
        listed[0].get("size_bytes").is_none(),
        "sizes are not reported yet"
    );

    repo.entity_mut()
        .warm_used(key, START.saturating_add(SignedDuration::from_secs(90)));
    commit_at(&repos, &mut repo, 90).await;
    let (_, used, _) = Call::new(Method::GET, &uri).send(&router).await;
    assert_eq!(used[0]["last_used_at"], at(90));

    let (status, _, _) = Call::new(Method::GET, "/v1/repos/repo_unknown/snapshots")
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn storage_reports_the_blobs_stored_and_no_sweep_before_the_first() {
    let router = api();
    let (status, empty, _) = Call::new(Method::GET, "/v1/storage").send(&router).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty, json!({ "blobs": 0, "bytes": 0, "snapshots": [] }));

    let id = snapshot(&router).await;
    let (_, _, manifest) = Call::new(Method::GET, format!("/v1/blobs/{id}"))
        .send(&router)
        .await;
    let (_, body, _) = Call::new(Method::GET, "/v1/storage").send(&router).await;
    assert_eq!(body["blobs"], json!(2), "the layer and the manifest");
    assert_eq!(body["bytes"], json!(LAYER.len() + manifest.len()));
    assert!(body.get("last_sweep").is_none());
    assert_eq!(body["snapshots"], json!([]));
}

#[tokio::test]
async fn workspaces_are_opened_listed_stopped_started_and_deleted() {
    let router = api();
    let dir = tempfile::tempdir().expect("dir");
    let origin = igloo_git::testing::Fixture::new(dir.path());
    origin.commit("base", &[("README.md", Some("igloo"))]);
    origin.switch("feature");
    origin.commit("change", &[("src/lib.rs", Some("v1"))]);
    let repo = register_origin(&router, &origin).await;

    let (status, workspace, _) = Call::new(Method::POST, format!("/v1/repos/{repo}/workspaces"))
        .json(&json!({ "branch": "feature" }))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{workspace}");
    assert_eq!(workspace["repo"], repo);
    assert_eq!(workspace["branch"], "feature");
    assert_eq!(workspace["phase"], "starting");
    assert!(
        workspace["owner"]
            .as_str()
            .expect("owner")
            .starts_with("usr_")
    );
    assert!(workspace.get("sandbox").is_none(), "no sandbox yet");
    let id = workspace["id"].as_str().expect("id");

    let (_, default, _) = Call::new(Method::POST, format!("/v1/repos/{repo}/workspaces"))
        .json(&json!({}))
        .send(&router)
        .await;
    assert_eq!(default["branch"], "main", "the default branch when unset");

    let (status, listed, _) = Call::new(Method::GET, "/v1/workspaces").send(&router).await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<_> = listed
        .as_array()
        .expect("list")
        .iter()
        .map(|workspace| workspace["id"].clone())
        .collect();
    assert_eq!(ids, [json!(id), default["id"].clone()], "oldest first");
    let (_, got, _) = Call::new(Method::GET, format!("/v1/workspaces/{id}"))
        .send(&router)
        .await;
    assert_eq!(got, workspace);

    let (status, stopped, _) = Call::new(Method::POST, format!("/v1/workspaces/{id}/stop"))
        .send(&router)
        .await;
    assert_eq!(
        (status, &stopped["phase"]),
        (StatusCode::OK, &json!("stopped"))
    );
    let (_, again, _) = Call::new(Method::POST, format!("/v1/workspaces/{id}/stop"))
        .send(&router)
        .await;
    assert_eq!(again["phase"], "stopped", "stopping twice changes nothing");
    let (_, started, _) = Call::new(Method::POST, format!("/v1/workspaces/{id}/start"))
        .send(&router)
        .await;
    assert_eq!(started["phase"], "starting");

    let (status, _, _) = Call::new(Method::DELETE, format!("/v1/workspaces/{id}"))
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    for call in [
        Call::new(Method::GET, format!("/v1/workspaces/{id}")),
        Call::new(Method::POST, format!("/v1/workspaces/{id}/start")),
        Call::new(Method::DELETE, format!("/v1/workspaces/{id}")),
    ] {
        let (status, body, _) = call.send(&router).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["code"], "workspace.not_found");
    }
    let (_, listed, _) = Call::new(Method::GET, "/v1/workspaces").send(&router).await;
    assert_eq!(listed.as_array().expect("list").len(), 1);
}

#[tokio::test]
async fn a_workspace_needs_an_existing_repository_and_branch() {
    let router = api();
    let dir = tempfile::tempdir().expect("dir");
    let origin = igloo_git::testing::Fixture::new(dir.path());
    origin.commit("base", &[("README.md", Some("igloo"))]);
    let repo = register_origin(&router, &origin).await;
    let missing = Id::<Repo>::from_uuid(Uuid::from_u128(99));

    for (uri, body, status, code) in [
        (
            format!("/v1/repos/{missing}/workspaces"),
            json!({}),
            StatusCode::NOT_FOUND,
            "repo.not_found",
        ),
        (
            format!("/v1/repos/{repo}/workspaces"),
            json!({ "branch": "nope" }),
            StatusCode::NOT_FOUND,
            "branch.not_found",
        ),
        (
            format!("/v1/repos/{repo}/workspaces"),
            json!({ "branch": "not a branch" }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation.invalid",
        ),
    ] {
        let (got, problem, _) = Call::new(Method::POST, uri).json(&body).send(&router).await;
        assert_eq!(
            (got, problem["code"].as_str()),
            (status, Some(code)),
            "{problem}"
        );
    }
    let (status, _, _) = Call::new(Method::GET, "/v1/workspaces")
        .anonymous()
        .send(&router)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
