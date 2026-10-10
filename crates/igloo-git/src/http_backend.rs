use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::error::GitError;
use crate::git::{Environment, Git};
use crate::invocation::Invocation;
use crate::refs::RefName;

/// The git service a smart HTTP request is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    /// Clones and fetches.
    UploadPack,
    /// Pushes.
    ReceivePack,
}

/// Which of the smart HTTP protocol's requests to serve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exchange {
    /// `GET info/refs?service=...`: the refs the service offers.
    Advertise(Service),
    /// `POST git-<service>`: one round of the service's conversation.
    Converse(Service),
}

/// One request for `git http-backend` to serve out of a repository directory.
///
/// Invariant: only the four requests of [`Exchange`] can be built; the dumb protocol and
/// arbitrary paths are not reachable.
#[derive(Clone, Debug)]
pub struct BackendRequest {
    exchange: Exchange,
    repository: String,
    user: String,
    protocol: Option<String>,
    gzip: bool,
    content_length: Option<u64>,
    refusal: Option<Refusal>,
}

/// Refs a push may not update, and what the pusher is told.
#[derive(Clone, Debug)]
struct Refusal {
    refs: Vec<RefName>,
    message: String,
}

/// Serves repositories under one root with `git http-backend`.
///
/// Invariant: git runs as [`Git::isolated`] does: no global or system configuration, no prompts,
/// and no hooks except [`HttpBackend::PRE_RECEIVE`] for pushes that refuse refs; refs under
/// `refs/igloo/` are neither offered nor writable; and `receive-pack` is enabled only for
/// requests naming a user.
#[derive(Clone, Debug)]
pub struct HttpBackend {
    git: Git,
    root: PathBuf,
}

/// A running `git http-backend`: its input and output speak CGI.
///
/// Invariant: git is killed when this is dropped.
#[derive(Debug)]
pub struct BackendProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
}

impl Service {
    /// The name git uses for it, such as `git-upload-pack`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::UploadPack => "git-upload-pack",
            Self::ReceivePack => "git-receive-pack",
        }
    }
}

impl BackendRequest {
    /// `exchange` for the repository directory named `repository` under the root, on behalf of
    /// `user`.
    ///
    /// `repository` must be a single path component: no separators, not `.` or `..`.
    pub fn new(
        exchange: Exchange,
        repository: &str,
        user: &str,
    ) -> Result<Self, crate::GitValueError> {
        let valid = !repository.is_empty()
            && !matches!(repository, "." | "..")
            && !repository.starts_with('-')
            && repository
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if !valid || user.is_empty() || user.contains(char::is_control) {
            return Err(crate::GitValueError::Path);
        }
        Ok(Self {
            exchange,
            repository: repository.to_owned(),
            user: user.to_owned(),
            protocol: None,
            gzip: false,
            content_length: None,
            refusal: None,
        })
    }

    /// A push updating any of `refs` is refused, whatever else it carries, and the pusher sees
    /// `message`, its control characters replaced by spaces. Only meaningful for pushes
    /// (`Converse(ReceivePack)`); call [`HttpBackend::prepare`] once before serving one.
    #[must_use]
    pub fn refusing(mut self, refs: Vec<RefName>, message: &str) -> Self {
        self.refusal = Some(Refusal {
            refs,
            message: message
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect(),
        });
        self
    }

    /// The `Git-Protocol` header the client sent, such as `version=2`; ignored unless it is
    /// made of ASCII alphanumerics and `=:;,._-`.
    #[must_use]
    pub fn with_protocol(mut self, protocol: &str) -> Self {
        let valid = protocol
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"=:;,._-".contains(&b));
        self.protocol = (valid && !protocol.is_empty()).then(|| protocol.to_owned());
        self
    }

    /// The request body is gzip-compressed; git inflates it.
    #[must_use]
    pub const fn gzipped(mut self) -> Self {
        self.gzip = true;
        self
    }

    /// The request body is `length` bytes; when unknown, git reads it to the end of its input.
    #[must_use]
    pub const fn with_content_length(mut self, length: u64) -> Self {
        self.content_length = Some(length);
        self
    }
}

impl HttpBackend {
    /// The hook refusing the refs a request names: it reads `old new ref` lines and fails,
    /// telling the pusher `$IGLOO_REFUSAL`, when `ref` is one of `$IGLOO_REFUSED_REFS`.
    pub const PRE_RECEIVE: &'static str = "#!/bin/sh\n\
        refused=0\n\
        while read -r old new ref; do\n\
        \tfor protected in $IGLOO_REFUSED_REFS; do\n\
        \t\tif [ \"$ref\" = \"$protected\" ]; then\n\
        \t\t\techo \"$IGLOO_REFUSAL\" >&2\n\
        \t\t\trefused=1\n\
        \t\tfi\n\
        \tdone\n\
        done\n\
        exit $refused\n";
    /// Where the hooks live, under the root.
    const HOOKS: &'static str = ".igloo-hooks";

    /// Serves the repositories directly under `root` with `git`.
    #[must_use]
    pub fn new(git: Git, root: impl Into<PathBuf>) -> Self {
        Self {
            git,
            root: root.into(),
        }
    }

    /// Writes [`Self::PRE_RECEIVE`] under the root; needed once before serving pushes that
    /// [`BackendRequest::refusing`] refs.
    pub async fn prepare(&self) -> Result<(), GitError> {
        use std::os::unix::fs::PermissionsExt;

        let dir = self.root.join(Self::HOOKS);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(GitError::Io)?;
        let hook = dir.join("pre-receive");
        tokio::fs::write(&hook, Self::PRE_RECEIVE)
            .await
            .map_err(GitError::Io)?;
        tokio::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
            .await
            .map_err(GitError::Io)
    }

    /// Starts git for `request`.
    pub fn spawn(&self, request: &BackendRequest) -> Result<BackendProcess, GitError> {
        let (method, path, query, content_type) = match request.exchange {
            Exchange::Advertise(service) => (
                "GET",
                format!("/{}/info/refs", request.repository),
                format!("service={}", service.name()),
                String::new(),
            ),
            Exchange::Converse(service) => (
                "POST",
                format!("/{}/{}", request.repository, service.name()),
                String::new(),
                format!("application/x-{}-request", service.name()),
            ),
        };
        let refusal = request
            .refusal
            .as_ref()
            .filter(|_| request.exchange == Exchange::Converse(Service::ReceivePack));
        let hooks = if refusal.is_some() {
            self.root.join(Self::HOOKS)
        } else {
            PathBuf::from("/dev/null")
        };
        let mut command = Command::new(self.git.program());
        command
            .arg("http-backend")
            .current_dir(&self.root)
            .env("LC_ALL", "C")
            .env("GIT_PROJECT_ROOT", &self.root)
            .env("GIT_HTTP_EXPORT_ALL", "1")
            .env("REQUEST_METHOD", method)
            .env("PATH_INFO", path)
            .env("QUERY_STRING", query)
            .env("CONTENT_TYPE", content_type)
            .env("REMOTE_USER", &request.user)
            .env("GIT_CONFIG_COUNT", "2")
            .env("GIT_CONFIG_KEY_0", "core.hooksPath")
            .env("GIT_CONFIG_VALUE_0", &hooks)
            .env("GIT_CONFIG_KEY_1", "transfer.hideRefs")
            .env("GIT_CONFIG_VALUE_1", "refs/igloo")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for variable in Invocation::REPOSITORY_VARIABLES {
            command.env_remove(variable);
        }
        if self.git.environment() == Environment::Isolated {
            command
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_TERMINAL_PROMPT", "0");
        }
        if let Some(protocol) = &request.protocol {
            command.env("GIT_PROTOCOL", protocol);
        }
        if let Some(refusal) = refusal {
            let refs: Vec<&str> = refusal.refs.iter().map(RefName::as_str).collect();
            command
                .env("IGLOO_REFUSED_REFS", refs.join(" "))
                .env("IGLOO_REFUSAL", &refusal.message);
        }
        if request.gzip {
            command.env("HTTP_CONTENT_ENCODING", "gzip");
        }
        if let Some(length) = request.content_length {
            command.env("CONTENT_LENGTH", length.to_string());
        }
        let mut child = command.spawn().map_err(GitError::Spawn)?;
        Ok(BackendProcess {
            stdin: child.stdin.take(),
            stdout: child.stdout.take(),
            child,
        })
    }
}

impl BackendProcess {
    /// git's input, to write the request body to; dropping it ends the body. Once only.
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.stdin.take()
    }

    /// git's output: a CGI response, its headers then its body. Once only.
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.stdout.take()
    }

    /// Waits for git to exit; fails with git's message when it did not exit with 0.
    pub async fn finish(mut self) -> Result<(), GitError> {
        drop(self.stdin.take());
        let mut stderr = String::new();
        if let Some(mut pipe) = self.child.stderr.take() {
            // The bound keeps a runaway git from filling memory.
            let mut limited = (&mut pipe).take(64 * 1024);
            limited
                .read_to_string(&mut stderr)
                .await
                .map_err(GitError::Io)?;
        }
        let status = self.child.wait().await.map_err(GitError::Io)?;
        if status.success() {
            Ok(())
        } else {
            Err(GitError::Failed {
                command: "http-backend",
                status: status.code(),
                stderr: stderr.trim().to_owned(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncReadExt;

    use super::*;
    use crate::git::Layout;
    use crate::testing::Fixture;

    #[test]
    fn repository_names_are_single_components() {
        let advertise = Exchange::Advertise(Service::UploadPack);
        for name in ["repo_x.git", "a-b.c"] {
            assert!(BackendRequest::new(advertise, name, "u").is_ok(), "{name}");
        }
        for name in ["", ".", "..", "../x", "a/b", "-x", "a b", "a\n"] {
            assert!(
                BackendRequest::new(advertise, name, "u").is_err(),
                "{name:?}"
            );
        }
        assert!(BackendRequest::new(advertise, "r.git", "").is_err());
    }

    #[test]
    fn protocol_headers_with_odd_characters_are_dropped() {
        let request = BackendRequest::new(Exchange::Advertise(Service::UploadPack), "r.git", "u")
            .expect("request");
        assert!(
            request
                .clone()
                .with_protocol("version=2")
                .protocol
                .is_some()
        );
        assert!(request.with_protocol("version=2\nX: y").protocol.is_none());
    }

    #[tokio::test]
    async fn advertises_refs_for_both_services() {
        let dir = tempfile::tempdir().expect("dir");
        let fixture = Fixture::new(dir.path());
        fixture.commit("first", &[("a", Some("a"))]);
        let git = Git::isolated();
        let backend = HttpBackend::new(git.clone(), dir.path());
        git.init(dir.path().join("served.git"), Layout::Bare, None)
            .await
            .expect("init");

        let request = BackendRequest::new(
            Exchange::Advertise(Service::UploadPack),
            "origin.git",
            "igloo",
        )
        .expect("request");
        let mut process = backend.spawn(&request).expect("spawn");
        let mut body = String::new();
        process
            .take_stdout()
            .expect("stdout")
            .read_to_string(&mut body)
            .await
            .expect("read");
        process.finish().await.expect("finish");
        assert!(body.contains("Content-Type: application/x-git-upload-pack-advertisement"));
        assert!(body.contains("refs/heads/main"), "{body}");
        let request = BackendRequest::new(
            Exchange::Advertise(Service::ReceivePack),
            "origin.git",
            "igloo",
        )
        .expect("request");
        let mut process = backend.spawn(&request).expect("spawn");
        let mut body = String::new();
        process
            .take_stdout()
            .expect("stdout")
            .read_to_string(&mut body)
            .await
            .expect("read");
        process.finish().await.expect("finish");
        assert!(
            body.contains("application/x-git-receive-pack-advertisement"),
            "{body}"
        );
    }

    async fn run_hook(refs: &str, pushed: &str) -> (bool, String) {
        let dir = tempfile::tempdir().expect("dir");
        let backend = HttpBackend::new(Git::isolated(), dir.path());
        backend.prepare().await.expect("prepare");
        let hook = dir.path().join(HttpBackend::HOOKS).join("pre-receive");
        let output = Command::new(hook)
            .env("IGLOO_REFUSED_REFS", refs)
            .env("IGLOO_REFUSAL", "use `igloo change merge`")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        let mut output = output;
        let mut stdin = output.stdin.take().expect("stdin");
        tokio::io::AsyncWriteExt::write_all(&mut stdin, pushed.as_bytes())
            .await
            .expect("write");
        drop(stdin);
        let done = output.wait_with_output().await.expect("wait");
        (
            done.status.success(),
            String::from_utf8_lossy(&done.stderr).into_owned(),
        )
    }

    #[tokio::test]
    async fn the_hook_refuses_only_the_named_refs() {
        let zero = "0".repeat(40);
        let one = "1".repeat(40);
        let (ok, said) = run_hook(
            "refs/heads/main",
            &format!("{zero} {one} refs/heads/topic\n"),
        )
        .await;
        assert!(ok, "{said}");
        let (ok, said) = run_hook(
            "refs/heads/main",
            &format!("{zero} {one} refs/heads/topic\n{one} {zero} refs/heads/main\n"),
        )
        .await;
        assert!(!ok);
        assert!(said.contains("igloo change merge"), "{said}");
    }

    #[tokio::test]
    async fn a_refusal_is_one_line_and_optional() {
        let dir = tempfile::tempdir().expect("dir");
        let backend = HttpBackend::new(Git::isolated(), dir.path());
        backend.prepare().await.expect("prepare");
        let push = BackendRequest::new(Exchange::Converse(Service::ReceivePack), "r.git", "u")
            .expect("request");
        let refusing = push
            .clone()
            .refusing(vec!["refs/heads/main".parse().expect("ref")], "no\nway");
        assert_eq!(
            refusing.refusal.as_ref().map(|r| r.message.as_str()),
            Some("no way")
        );
        assert!(push.refusal.is_none());
    }
}
