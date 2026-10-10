//! `igloo shell` against a fake server, under a pseudo-terminal: it attaches, follows resizes,
//! starts a stopped workspace, chooses the repository's latest workspace and exits with the
//! remote shell's exit code.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "a test reports failures by panicking"
)]
#![allow(
    clippy::print_stderr,
    reason = "a failing test shows what the CLI printed"
)]

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read as _, Write as _};
use std::os::unix::ffi::OsStrExt as _;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use igloo_git::testing::Fixture;
use rustix::process::{Pid, Signal};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{self, LocalModes, Winsize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

/// What the fake server was asked, in order: `METHOD /path`, and for the terminal the query and
/// whether it carried the token.
type Log = Arc<Mutex<Vec<String>>>;

/// The fake Igloo: scripted workspaces and a terminal that echoes.
struct Fake {
    log: Log,
    /// Phases returned by successive `GET /v1/workspaces/wsp_a`; the last repeats.
    phases: Mutex<VecDeque<&'static str>>,
    workspaces: Value,
    repos: Value,
}

fn workspace(id: &str, repo: &str, phase: &str, active: &str) -> Value {
    let mut workspace = json!({
        "id": id, "owner": "usr_1", "repo": repo, "branch": "main", "phase": phase,
        "last_activity": active, "created_at": "2026-10-01T00:00:00Z",
    });
    if phase == "running" {
        workspace["sandbox"] = json!("sbx_1");
    }
    workspace
}

impl Fake {
    fn new(phases: &[&'static str]) -> Arc<Self> {
        Arc::new(Self {
            log: Log::default(),
            phases: Mutex::new(phases.iter().copied().collect()),
            workspaces: json!([]),
            repos: json!([]),
        })
    }

    fn with_listing(workspaces: Value, repos: Value, phases: &[&'static str]) -> Arc<Self> {
        Arc::new(Self {
            log: Log::default(),
            phases: Mutex::new(phases.iter().copied().collect()),
            workspaces,
            repos,
        })
    }

    fn next_phase(&self) -> &'static str {
        let mut phases = self.phases.lock().expect("lock");
        if phases.len() > 1 {
            phases.pop_front().expect("a phase")
        } else {
            phases.front().copied().expect("a phase")
        }
    }

    fn record(&self, line: impl Into<String>) {
        self.log.lock().expect("lock").push(line.into());
    }

    async fn serve(self: Arc<Self>, listener: TcpListener) {
        while let Ok((stream, _)) = listener.accept().await {
            self.connection(stream).await;
        }
    }

    /// The request head, without consuming it.
    async fn peek_head(stream: &TcpStream) -> String {
        let mut buffer = vec![0_u8; 8192];
        loop {
            let read = stream.peek(&mut buffer).await.expect("peek");
            let text = String::from_utf8_lossy(&buffer[..read]).into_owned();
            if text.contains("\r\n\r\n") {
                return text;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    async fn connection(&self, mut stream: TcpStream) {
        let head = Self::peek_head(&stream).await;
        let line = head.lines().next().unwrap_or_default().to_owned();
        let mut parts = line.split(' ');
        let (method, path) = (
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default().to_owned(),
        );
        if path.contains("/terminal") {
            self.terminal(stream, &path).await;
            return;
        }
        // Consume the head that was only peeked at.
        let mut consumed = vec![0_u8; head.len()];
        stream.read_exact(&mut consumed).await.expect("head");
        self.record(format!("{method} {path}"));
        let body = match (method, path.as_str()) {
            ("GET", "/v1/workspaces") => self.workspaces.clone(),
            ("GET", "/v1/repos") => self.repos.clone(),
            ("GET" | "POST", "/v1/workspaces/wsp_a" | "/v1/workspaces/wsp_a/start") => {
                workspace("wsp_a", "repo_1", self.next_phase(), "2026-10-05T00:00:00Z")
            }
            ("GET" | "POST", "/v1/workspaces/wsp_b" | "/v1/workspaces/wsp_b/start") => {
                workspace("wsp_b", "repo_1", "running", "2026-10-06T00:00:00Z")
            }
            _ => json!({ "title": "no such route", "status": 404, "code": "fake.not_found" }),
        };
        let status = if body.get("code").is_some() { 404 } else { 200 };
        let body = body.to_string();
        let response = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n\
             content-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.expect("write");
    }

    /// A terminal that greets with the size it was opened with, echoes input, reports resizes
    /// and exits with 7 when it is told `exit`.
    async fn terminal(&self, stream: TcpStream, path: &str) {
        let log = Arc::clone(&self.log);
        #[allow(
            clippy::result_large_err,
            reason = "the handshake callback's signature is tungstenite's"
        )]
        let callback = move |request: &Request, mut response: Response| {
            let token = request
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                == Some("Bearer t");
            log.lock()
                .expect("lock")
                .push(format!("TERMINAL {} token={token}", request.uri()));
            response.headers_mut().insert(
                "sec-websocket-protocol",
                "igloo.terminal.v1".parse().expect("header"),
            );
            Ok(response)
        };
        let mut socket = tokio_tungstenite::accept_hdr_async(stream, callback)
            .await
            .expect("handshake");
        let query = path.split_once('?').map_or("", |(_, query)| query);
        let size = |name: &str| {
            query
                .split('&')
                .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
                .unwrap_or("?")
        };
        let greeting = format!("ready {}x{}\r\n", size("cols"), size("rows"));
        socket
            .send(Message::Binary(greeting.into_bytes().into()))
            .await
            .expect("greet");
        while let Some(Ok(message)) = socket.next().await {
            match message {
                Message::Binary(data) if data.windows(4).any(|word| word == b"exit") => {
                    let exit = Message::Text(r#"{"type":"exit","code":7}"#.into());
                    socket.send(exit).await.expect("exit");
                    socket.close(None).await.ok();
                    return;
                }
                Message::Binary(data) => {
                    let mut echo = b"echo:".to_vec();
                    echo.extend_from_slice(&data);
                    socket
                        .send(Message::Binary(echo.into()))
                        .await
                        .expect("echo");
                }
                Message::Text(text) => {
                    let frame: Value = serde_json::from_str(text.as_str()).expect("frame");
                    let said = format!("resized {}x{}\r\n", frame["cols"], frame["rows"]);
                    socket
                        .send(Message::Binary(said.into_bytes().into()))
                        .await
                        .expect("report");
                }
                _ => {}
            }
        }
    }
}

/// `igloo shell` running in a pseudo-terminal.
struct Shell {
    child: Child,
    master: File,
    slave: File,
    output: mpsc::UnboundedReceiver<Vec<u8>>,
    seen: String,
}

impl Shell {
    fn open_terminal() -> (File, File) {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).expect("openpt");
        grantpt(&master).expect("grantpt");
        unlockpt(&master).expect("unlockpt");
        let name = ptsname(&master, Vec::new()).expect("ptsname");
        let slave = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(std::ffi::OsStr::from_bytes(name.as_bytes()))
            .expect("slave");
        (File::from(master), slave)
    }

    /// Runs `igloo shell` with `arguments` in `dir`, on a screen of `cols` by `rows`.
    fn start(
        address: &str,
        dir: &std::path::Path,
        arguments: &[&str],
        cols: u16,
        rows: u16,
    ) -> Self {
        let (master, slave) = Self::open_terminal();
        Self::resize_screen(&slave, cols, rows);
        let mut command = Command::new(env!("CARGO_BIN_EXE_igloo"));
        command
            .args([
                "--api",
                &format!("http://{address}"),
                "--token",
                "t",
                "shell",
            ])
            .args(arguments)
            .current_dir(dir)
            .env_remove("IGLOO_API")
            .env_remove("IGLOO_TOKEN")
            .stdin(Stdio::from(slave.try_clone().expect("stdin")))
            .stdout(Stdio::from(slave.try_clone().expect("stdout")))
            .stderr(Stdio::from(slave.try_clone().expect("stderr")));
        let child = command.spawn().expect("igloo");
        let (sender, output) = mpsc::unbounded_channel();
        let mut reader = master.try_clone().expect("reader");
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 || sender.send(buffer[..read].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            master,
            slave,
            output,
            seen: String::new(),
        }
    }

    fn resize_screen(terminal: &File, cols: u16, rows: u16) {
        let size = Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        termios::tcsetwinsize(terminal, size).expect("winsize");
    }

    /// Reads the terminal until it shows `text`.
    async fn expect(&mut self, text: &str) {
        let wait = async {
            while !self.seen.contains(text) {
                let chunk = self.output.recv().await.expect("the terminal ended");
                self.seen.push_str(&String::from_utf8_lossy(&chunk));
            }
        };
        if tokio::time::timeout(Duration::from_secs(20), wait)
            .await
            .is_err()
        {
            panic!("never saw {text:?}; saw {:?}", self.seen);
        }
    }

    fn type_in(&mut self, text: &str) {
        self.master.write_all(text.as_bytes()).expect("type");
    }

    fn raw(&self) -> bool {
        let modes = termios::tcgetattr(&self.slave)
            .expect("attributes")
            .local_modes;
        !modes.intersects(LocalModes::ICANON | LocalModes::ECHO | LocalModes::ISIG)
    }

    fn screen_resized(&mut self, cols: u16, rows: u16) {
        Self::resize_screen(&self.slave, cols, rows);
        let pid = Pid::from_child(&self.child);
        rustix::process::kill_process(pid, Signal::WINCH).expect("signal");
    }

    /// The exit code, once the CLI ended.
    async fn code(&mut self) -> Option<i32> {
        for _ in 0..400 {
            if let Some(status) = self.child.try_wait().expect("wait") {
                return status.code();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("igloo shell did not exit; saw {:?}", self.seen);
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            self.child.kill().ok();
            self.child.wait().ok();
        }
        if std::thread::panicking() {
            eprintln!("terminal showed: {:?}", self.seen);
        }
    }
}

async fn bind() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address").to_string();
    (listener, address)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shell_attaches_in_raw_mode_follows_resizes_and_exits_with_the_remote_code() {
    let fake = Fake::new(&["running"]);
    let (listener, address) = bind().await;
    let dir = tempfile::tempdir().expect("dir");
    let scenario = async {
        let mut shell = Shell::start(&address, dir.path(), &["wsp_a"], 100, 30);
        shell.expect("ready 100x30").await;
        assert!(shell.raw(), "the local terminal is raw while attached");

        shell.type_in("hi\r");
        shell.expect("echo:hi\r").await;

        shell.screen_resized(120, 40);
        shell.expect("resized 120x40").await;

        shell.type_in("exit\r");
        assert_eq!(shell.code().await, Some(7), "the remote shell's exit code");
        assert!(!shell.raw(), "the terminal's mode is restored on exit");
    };
    tokio::select! {
        () = Arc::clone(&fake).serve(listener) => {}
        () = scenario => {}
    }
    let log = fake.log.lock().expect("lock");
    assert!(
        log.iter().any(|line| line
            .contains("TERMINAL /v1/sandboxes/sbx_1/terminal?cols=100&rows=30 token=true")),
        "{log:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stopped_workspace_is_started_and_waited_for_before_the_terminal_opens() {
    let fake = Fake::new(&["stopped", "starting", "running"]);
    let (listener, address) = bind().await;
    let dir = tempfile::tempdir().expect("dir");
    let scenario = async {
        let mut shell = Shell::start(&address, dir.path(), &["wsp_a"], 80, 24);
        shell
            .expect("workspace wsp_a is not running; starting it")
            .await;
        shell.expect("ready 80x24").await;
        shell.type_in("exit\r");
        assert_eq!(shell.code().await, Some(7));
    };
    tokio::select! {
        () = Arc::clone(&fake).serve(listener) => {}
        () = scenario => {}
    }
    let log = fake.log.lock().expect("lock").clone();
    let position = |needle: &str| log.iter().position(|line| line.contains(needle));
    let started = position("POST /v1/workspaces/wsp_a/start").expect("started");
    let terminal = position("TERMINAL").expect("terminal");
    assert!(started < terminal, "{log:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_an_argument_shell_opens_the_latest_workspace_of_the_current_repository() {
    let dir = tempfile::tempdir().expect("dir");
    let fixture = Fixture::new(dir.path());
    fixture.commit("first", &[("a", Some("a"))]);
    let origin = fixture.origin().display().to_string();
    fixture.git(&["remote", "add", "origin", &origin]);
    let repos = json!([
        { "id": "repo_1", "location": origin, "default_branch": "main", "git_path": "/git/repo_1.git" },
        { "id": "repo_2", "location": "github.com/o/other", "default_branch": "main", "git_path": "/git/repo_2.git" },
    ]);
    let workspaces = json!([
        workspace("wsp_a", "repo_1", "running", "2026-10-05T00:00:00Z"),
        workspace("wsp_b", "repo_1", "running", "2026-10-06T00:00:00Z"),
        workspace("wsp_c", "repo_2", "running", "2026-10-09T00:00:00Z"),
    ]);
    let fake = Fake::with_listing(workspaces, repos, &["running"]);
    let (listener, address) = bind().await;
    let scenario = async {
        let mut shell = Shell::start(&address, fixture.work(), &[], 80, 24);
        shell.expect("ready 80x24").await;
        shell.type_in("exit\r");
        assert_eq!(shell.code().await, Some(7));
    };
    tokio::select! {
        () = Arc::clone(&fake).serve(listener) => {}
        () = scenario => {}
    }
    let log = fake.log.lock().expect("lock").clone();
    assert!(
        log.iter().any(|line| line == "GET /v1/workspaces/wsp_b"),
        "the repository's most recently used workspace: {log:?}"
    );
    assert!(!log.iter().any(|line| line.contains("wsp_c")), "{log:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shell_without_a_workspace_says_how_to_open_one() {
    let dir = tempfile::tempdir().expect("dir");
    let fixture = Fixture::new(dir.path());
    fixture.commit("first", &[("a", Some("a"))]);
    let origin = fixture.origin().display().to_string();
    fixture.git(&["remote", "add", "origin", &origin]);
    let repos = json!([{ "id": "repo_1", "location": origin, "default_branch": "main", "git_path": "/git/repo_1.git" }]);
    let fake = Fake::with_listing(json!([]), repos, &["running"]);
    let (listener, address) = bind().await;
    let scenario = async {
        let mut shell = Shell::start(&address, fixture.work(), &[], 80, 24);
        shell.expect("igloo workspace create").await;
        assert_eq!(shell.code().await, Some(2));
    };
    tokio::select! {
        () = fake.serve(listener) => {}
        () = scenario => {}
    }
}
