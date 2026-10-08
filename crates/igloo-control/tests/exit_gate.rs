//! The phase 2 exit gate, items 1 and 2: an imported Rust image runs `cargo test` in an
//! isolated container, and a fork of a warm snapshot starts in under a second without
//! recompiling dependencies.
//!
//! Linux, as root, with an OCI runtime. Runs when `IGLOO_TEST_EXIT_GATE=1`; reads
//! `IGLOO_TEST_OCI_RUNTIME` (default `youki`), `IGLOO_TEST_IMAGE` (default
//! `docker.io/library/rust:1.99-slim`) and `IGLOO_TEST_DATABASE_URL` (default: a Postgres
//! container).

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "the helpers of a test report failures by panicking"
)]

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use igloo::job::ExecRequest;
use igloo::sandbox::{CreateSandboxRequest, Isolation, SandboxPhase, SandboxResource};
use igloo::snapshot::{CreateSnapshotRequest, ImportImageRequest, Layer, MediaType};
use igloo::{Client, Layer as DirLayer};
use igloo_control::{Config, Server};
use igloo_worker::{Worker, WorkerConfig};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use self::common::{Database, Output};

mod common;

const DEV_TOKEN: &str = "dev-token";
const JOIN_TOKEN: &str = "join-token";

/// A crate with a path dependency: a fork that recompiles nothing prints no `Compiling dep`.
const PROJECT: [(&str, &str); 4] = [
    (
        "Cargo.toml",
        "[package]\nname = \"gate\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
         [dependencies]\ndep = { path = \"dep\" }\n",
    ),
    (
        "src/lib.rs",
        "pub fn answer() -> u32 { dep::value() }\n\n\
         #[test]\nfn answers() { assert_eq!(answer(), 42); }\n",
    ),
    (
        "dep/Cargo.toml",
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    ),
    ("dep/src/lib.rs", "pub fn value() -> u32 { 42 }\n"),
];

struct Gate {
    _database: Database,
    _data: tempfile::TempDir,
    server: Server,
    workers: JoinSet<()>,
    stop: CancellationToken,
    client: Client,
}

impl Gate {
    async fn start() -> Self {
        let database = Database::start().await;
        let data = tempfile::tempdir().expect("data dir");
        let config = Config::from_vars([
            ("IGLOO_LISTEN", OsString::from("127.0.0.1:0")),
            ("IGLOO_GATEWAY_LISTEN", "127.0.0.1:0".into()),
            ("IGLOO_DATABASE_URL", database.url.clone().into()),
            ("IGLOO_DATA_DIR", data.path().join("server").into()),
            ("IGLOO_DEV_TOKEN", DEV_TOKEN.into()),
            ("IGLOO_JOIN_TOKEN", JOIN_TOKEN.into()),
            (
                "IGLOO_SECRETS_KEY",
                "end-to-end-secrets-key-0123456789abcd".into(),
            ),
            (
                "IGLOO_BLOB_KEY",
                "exit-gate-blob-signing-secret-0123456".into(),
            ),
        ])
        .expect("config");
        let server = Server::start(&config).await.expect("server");
        let worker = WorkerConfig::from_vars([
            (
                "IGLOO_WORKER_SERVER",
                OsString::from(format!("http://{}", server.gateway_address())),
            ),
            ("IGLOO_WORKER_JOIN_TOKEN", JOIN_TOKEN.into()),
            ("IGLOO_WORKER_DATA_DIR", data.path().join("worker").into()),
            ("IGLOO_WORKER_RUNTIME", "oci".into()),
            ("IGLOO_WORKER_OVERLAY", "true".into()),
            (
                "IGLOO_WORKER_OCI_RUNTIME",
                std::env::var_os("IGLOO_TEST_OCI_RUNTIME").unwrap_or_else(|| "youki".into()),
            ),
        ])
        .expect("worker config");
        let worker = Worker::new(worker).await.expect("worker");
        let stop = CancellationToken::new();
        let mut workers = JoinSet::new();
        let token = stop.clone();
        workers.spawn(async move {
            worker.run(token).await.expect("worker run");
        });
        let client =
            Client::new(&format!("http://{}", server.rest_address()), DEV_TOKEN).expect("client");
        Self {
            _database: database,
            _data: data,
            server,
            workers,
            stop,
            client,
        }
    }

    /// The image, imported for this machine's architecture.
    async fn import(&self) -> String {
        let image = std::env::var("IGLOO_TEST_IMAGE")
            .unwrap_or_else(|_| "docker.io/library/rust:1.99-slim".to_owned());
        let platform = if cfg!(target_arch = "aarch64") {
            "linux/arm64"
        } else {
            "linux/amd64"
        };
        let request = ImportImageRequest::new(image).with_platform(platform);
        tokio::time::timeout(Duration::from_secs(600), self.client.import_image(&request))
            .await
            .expect("imported within 10 minutes")
            .expect("import")
            .id
    }

    /// The project as a snapshot over `base`.
    async fn project(&self, base: String, dir: &Path) -> String {
        for (path, contents) in PROJECT {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, contents).expect("write");
        }
        let layer = DirLayer::from_dir(dir).expect("pack");
        let digest = self
            .client
            .upload_blob(layer.into_bytes())
            .await
            .expect("upload");
        let request =
            CreateSnapshotRequest::new(vec![Layer::new(digest, MediaType::Tar)]).with_base(base);
        self.client
            .create_snapshot(&request)
            .await
            .expect("snapshot")
            .id
    }

    /// An isolated sandbox from `snapshot`, once running; returns it and how long it took.
    async fn sandbox(&self, snapshot: String) -> (SandboxResource, Duration) {
        let request = CreateSandboxRequest::new(snapshot).with_isolation(Isolation::Container);
        let started = Instant::now();
        let sandbox = self.client.create_sandbox(&request).await.expect("create");
        loop {
            let current = self.client.get_sandbox(&sandbox.id).await.expect("get");
            match current.phase {
                SandboxPhase::Running => return (current, started.elapsed()),
                SandboxPhase::Failed | SandboxPhase::Stopped => {
                    panic!("the sandbox ended: {current:?}")
                }
                _ => {}
            }
            assert!(
                started.elapsed() < Duration::from_secs(600),
                "the sandbox started"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn exec(&self, sandbox: &str, script: &str) -> Output {
        let request = ExecRequest::new(vec!["sh".to_owned(), "-c".to_owned(), script.to_owned()]);
        let job = self.client.exec(sandbox, &request).await.expect("exec");
        let mut logs = self.client.logs(&job.id).await.expect("logs");
        Output::of(&mut logs, Duration::from_secs(600)).await
    }

    async fn stop(mut self) {
        self.stop.cancel();
        while self.workers.join_next().await.is_some() {}
        self.server
            .shutdown(Duration::from_secs(10))
            .await
            .expect("shutdown");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_imported_rust_image_tests_in_isolation_and_a_warm_fork_starts_fast() {
    if std::env::var_os("IGLOO_TEST_EXIT_GATE").is_none() {
        return;
    }
    let gate = Gate::start().await;
    let project = tempfile::tempdir().expect("project");
    let image = gate.import().await;
    let snapshot = gate.project(image, project.path()).await;
    let (sandbox, _) = gate.sandbox(snapshot).await;

    let host_only = project.path().join("Cargo.toml");
    let up = "for i in /sys/class/net/*; do [ \"$(cat $i/operstate)\" = up ] && basename $i; done";
    let script = format!(
        "set -e; test ! -e {}; test -z \"$({up})\"; cargo test --offline; cargo build --tests --offline",
        host_only.display()
    );
    let first = gate.exec(&sandbox.id, &script).await;
    assert_eq!(
        first.exit_code(),
        Some(0),
        "isolated cargo test: {}{}",
        first.stdout,
        first.stderr
    );
    assert!(first.stderr.contains("Compiling dep"), "{}", first.stderr);

    let sealed = gate.client.seal(&sandbox.id).await.expect("seal");
    let (fork, elapsed) = gate.sandbox(sealed).await;
    assert!(
        elapsed < Duration::from_secs(1),
        "the fork started in {elapsed:?}"
    );
    let second = gate.exec(&fork.id, "cargo test --offline").await;
    assert_eq!(second.exit_code(), Some(0), "{}", second.stderr);
    assert!(
        !second.stderr.contains("Compiling dep"),
        "the fork reuses the warm build: {}",
        second.stderr
    );
    gate.stop().await;
}

/// Igloo's own pipeline on a change to Igloo: the working tree as `main`, one more commit as the
/// change. Slow: imports the Rust image and builds the workspace. Runs when `IGLOO_TEST_SELF=1`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn igloos_own_pipeline_passes_on_a_change_to_igloo() {
    if std::env::var_os("IGLOO_TEST_SELF").is_none() {
        return;
    }
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let gate = Gate::start().await;
    let origin = tempfile::tempdir().expect("origin");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let git_dir = origin.path().join("work.git");
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("--git-dir")
            .arg(&git_dir)
            .arg("--work-tree")
            .arg(&source)
            .args([
                "-c",
                "user.name=igloo",
                "-c",
                "user.email=igloo@example.com",
            ])
            .args(args)
            .output()
            .expect("git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    git(&["init", "--quiet", "--initial-branch=main"]);
    std::process::Command::new("git")
        .args(["init", "--quiet", "--bare", "--initial-branch=main"])
        .arg(origin.path().join("origin.git"))
        .status()
        .expect("bare origin");
    git(&["add", "--all"]);
    git(&["commit", "--quiet", "-m", "Igloo"]);
    let bare = origin.path().join("origin.git").display().to_string();
    git(&["push", "--quiet", &format!("file://{bare}"), "main"]);
    git(&["commit", "--quiet", "--allow-empty", "-m", "A change"]);
    git(&[
        "push",
        "--quiet",
        &format!("file://{bare}"),
        "HEAD:refs/heads/change",
    ]);

    let repo = gate
        .client
        .register_repo(&igloo::repo::RegisterRepoRequest::new(bare))
        .await
        .expect("register");
    let change = gate
        .client
        .open_change(
            &repo.id,
            &igloo::change::OpenChangeRequest::new("change", "A change"),
        )
        .await
        .expect("open");
    let run = loop {
        let runs = gate.client.list_runs(&change.id).await.expect("runs");
        if let Some(run) = runs.into_iter().find(|run| {
            matches!(
                run.phase,
                igloo::ci::RunPhase::Passed
                    | igloo::ci::RunPhase::Failed
                    | igloo::ci::RunPhase::Errored
            )
        }) {
            break run;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    };
    let mut report = format!("{run:?}\n");
    let jobs = run.warm_job.iter().map(|job| ("warm", job)).chain(
        run.checks
            .iter()
            .filter_map(|check| Some((check.name.as_str(), check.job.as_ref()?))),
    );
    for (name, job) in jobs {
        let mut logs = gate.client.logs(job).await.expect("logs");
        let output = Output::of(&mut logs, Duration::from_secs(60)).await;
        let _ = writeln!(report, "--- {name} ---\n{}{}", output.stdout, output.stderr);
    }
    assert_eq!(run.phase, igloo::ci::RunPhase::Passed, "{report}");
    gate.stop().await;
}

/// A task with real Claude Code, in a container from a Debian image with Claude Code installed
/// into the agent snapshot: it commits a file, its change passes its check, and its transcript
/// holds Claude Code's tool calls. Runs when `IGLOO_TEST_AGENT=1`, with the subscription token
/// of `claude setup-token` in `IGLOO_TEST_CLAUDE_TOKEN`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn claude_code_works_on_a_task_in_a_container() {
    if std::env::var_os("IGLOO_TEST_AGENT").is_none() {
        return;
    }
    let token = std::env::var("IGLOO_TEST_CLAUDE_TOKEN").expect("IGLOO_TEST_CLAUDE_TOKEN");
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let gate = Gate::start().await;
    let client = &gate.client;
    let origin = self::common::Origin::new();
    origin.commit(&[
        (
            ".igloo/pipeline.toml",
            "image = \"docker.io/library/debian:bookworm-slim\"\n\n[sandbox]\n\
             isolation = \"container\"\n\n[[checks]]\nname = \"hello\"\n\
             run = \"grep -q hello hello.txt\"\n",
        ),
        (
            ".igloo/agents.toml",
            "default_tool = \"claude-code\"\n\n[tools.claude-code]\nharness = \"claude-code\"\n\
             install = \"apt-get update -qq && apt-get install -qq -y --no-install-recommends \
             curl ca-certificates git >/dev/null && curl -fsSL https://claude.ai/install.sh | bash \
             && ln -sf /root/.local/bin/claude /usr/local/bin/claude\"\n\
             secrets = [\"CLAUDE_CODE_OAUTH_TOKEN\"]\ntimeout_seconds = 900\n",
        ),
        ("README.md", "A test repository.\n"),
    ]);
    let repo = client
        .register_repo(&igloo::repo::RegisterRepoRequest::new(origin.path.clone()))
        .await
        .expect("register");
    client
        .set_secret(&repo.id, "CLAUDE_CODE_OAUTH_TOKEN", &token)
        .await
        .expect("secret");
    let task = client
        .create_task(
            &repo.id,
            &igloo::task::CreateTaskRequest::new(
                "Create a file named hello.txt containing the word hello. Do not commit.",
            ),
        )
        .await
        .expect("create");
    let deadline = Instant::now() + Duration::from_mins(30);
    let task = loop {
        let task = client.get_task(&task.id).await.expect("task");
        let busy = matches!(
            task.phase,
            igloo::task::TaskPhase::Preparing | igloo::task::TaskPhase::Working
        );
        if !busy || Instant::now() > deadline {
            break task;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    };
    let transcript = client
        .get_transcript(&task.id, 0)
        .await
        .expect("transcript");
    let report = format!(
        "{task:?}\n{:#?}\n{}",
        transcript.entries,
        gate.build_logs().await
    );
    assert_eq!(
        task.phase,
        igloo::task::TaskPhase::AwaitingReview,
        "{report}"
    );
    assert!(
        transcript
            .entries
            .iter()
            .any(|entry| matches!(entry.item, igloo::task::TranscriptItem::ToolCall { .. })),
        "Claude Code's tool calls are read: {report}"
    );
    assert!(!report.contains(&token), "the token never shows");
    let run = self::common::ended_run(client, &task.change.expect("change"), 1).await;
    assert_eq!(run.phase, igloo::ci::RunPhase::Passed, "{run:?}");
    gate.stop().await;
}

impl Gate {
    /// The output of every build, for reports.
    async fn build_logs(&self) -> String {
        let builds: Vec<String> = sqlx::query_scalar(
            "SELECT data->>'job' FROM events WHERE type = 'igloo.build.started' ORDER BY sequence",
        )
        .fetch_all(self.server.database().pool())
        .await
        .expect("builds");
        let mut report = String::new();
        for job in builds {
            let mut logs = self.client.logs(&job).await.expect("logs");
            let output = Output::of(&mut logs, Duration::from_secs(60)).await;
            let _ = writeln!(
                report,
                "--- build {job} ---\n{}{}",
                output.stdout, output.stderr
            );
        }
        report
    }
}
