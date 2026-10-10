use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

use crate::error::GitError;
use crate::git::{Environment, Git};

/// One run of git in a directory, built up and then run once.
pub(crate) struct Invocation<'a> {
    git: &'a Git,
    dir: &'a Path,
    command: &'static str,
    args: Vec<OsString>,
    config: Vec<(String, String)>,
    env: Vec<(&'static str, String)>,
}

/// What a finished invocation printed and how it exited.
pub(crate) struct Output {
    pub(crate) command: &'static str,
    pub(crate) dir: PathBuf,
    /// The exit code; `None` when killed by a signal.
    pub(crate) status: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: String,
}

impl<'a> Invocation<'a> {
    /// Repository location variables git must not take from the caller's environment.
    pub(crate) const REPOSITORY_VARIABLES: [&'static str; 8] = [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
        "GIT_PREFIX",
    ];

    /// `git -C <dir> <command>`.
    pub(crate) fn new(git: &'a Git, dir: &'a Path, command: &'static str) -> Self {
        Self {
            git,
            dir,
            command,
            args: Vec::new(),
            config: Vec::new(),
            env: Vec::new(),
        }
    }

    /// Appends one argument.
    pub(crate) fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    /// Sets environment variable `key` to `value` for this run.
    pub(crate) fn env(mut self, key: &'static str, value: impl Into<String>) -> Self {
        self.env.push((key, value.into()));
        self
    }

    /// Sets configuration `key` to `value` for this run, through git's environment.
    pub(crate) fn config(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.config.push((key.into(), value.into()));
        self
    }

    /// Runs git and returns its output when it exits with 0; otherwise the classified failure.
    pub(crate) async fn run(self) -> Result<Output, GitError> {
        let output = self.output().await?;
        if output.status == Some(0) {
            Ok(output)
        } else {
            Err(output.into_error())
        }
    }

    /// Runs git and returns its output, whatever its exit code.
    pub(crate) async fn output(mut self) -> Result<Output, GitError> {
        let mut command = Command::new(self.git.program());
        command
            .arg("-C")
            .arg(self.dir)
            .arg(self.command)
            .args(&self.args)
            .env("LC_ALL", "C")
            .env("GIT_LITERAL_PATHSPECS", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for variable in Self::REPOSITORY_VARIABLES {
            command.env_remove(variable);
        }
        for (key, value) in &self.env {
            command.env(key, value);
        }
        if self.git.environment() == Environment::Isolated {
            command
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_ALLOW_PROTOCOL", "file:http:https:ssh")
                .env_remove("GIT_ASKPASS")
                .env_remove("SSH_ASKPASS");
            self.config
                .push(("core.hooksPath".into(), "/dev/null".into()));
            self.config
                .push(("credential.helper".into(), String::new()));
        }
        command.env("GIT_CONFIG_COUNT", self.config.len().to_string());
        for (index, (key, value)) in self.config.iter().enumerate() {
            command
                .env(format!("GIT_CONFIG_KEY_{index}"), key)
                .env(format!("GIT_CONFIG_VALUE_{index}"), value);
        }
        let timeout = self.git.timeout();
        let output = tokio::time::timeout(timeout, command.output())
            .await
            .map_err(|_| GitError::Timeout {
                command: self.command,
                timeout,
            })?
            .map_err(GitError::Spawn)?;
        let output = Output {
            command: self.command,
            dir: self.dir.to_owned(),
            status: output.status.code(),
            stdout: output.stdout,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        };
        tracing::debug!(
            command = output.command,
            dir = %output.dir.display(),
            status = ?output.status,
            "git finished"
        );
        Ok(output)
    }
}

impl Output {
    /// stdout as UTF-8, without its trailing newline.
    pub(crate) fn text(&self) -> Result<&str, GitError> {
        std::str::from_utf8(&self.stdout)
            .map(|text| text.trim_end_matches('\n'))
            .map_err(|_| self.unexpected("output is not UTF-8"))
    }

    /// An [`GitError::Unexpected`] about this output.
    pub(crate) fn unexpected(&self, detail: impl Into<String>) -> GitError {
        GitError::Unexpected {
            command: self.command,
            detail: detail.into(),
        }
    }

    /// The failure this output reports, classified from git's (C locale) messages.
    pub(crate) fn into_error(self) -> GitError {
        const AUTHENTICATION: [&str; 5] = [
            "Authentication failed",
            "could not read Username",
            "could not read Password",
            "terminal prompts disabled",
            "Permission denied (publickey",
        ];
        let stderr = &self.stderr;
        if stderr.contains("not a git repository") || stderr.contains("cannot change to") {
            GitError::NotARepository(self.dir)
        } else if stderr.contains("couldn't find remote ref") {
            GitError::RemoteRefNotFound
        } else if AUTHENTICATION
            .iter()
            .any(|message| stderr.contains(message))
        {
            GitError::Authentication
        } else {
            GitError::Failed {
                command: self.command,
                status: self.status,
                stderr: self.stderr,
            }
        }
    }
}
