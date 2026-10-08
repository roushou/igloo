use std::collections::BTreeMap;
use std::path::Path;

use igloo_api::job::{ExecRequest, JobResource};
use igloo_api::sandbox::{CreateSandboxRequest, Isolation, SandboxResource};
use igloo_api::snapshot::{CreateSnapshotRequest, Layer, MediaType};

use crate::archive;
use crate::client::{Client, Error};
use crate::logs::LogStream;

/// A command to run against a local directory.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Run {
    /// The program and its arguments.
    pub argv: Vec<String>,
    /// Labels for the sandbox.
    pub labels: BTreeMap<String, String>,
    /// How long the command may run; the server default when unset.
    pub timeout_seconds: Option<u32>,
    /// The snapshot the directory is layered over, such as an imported image; none when unset.
    pub base: Option<String>,
    /// How strongly the sandbox is separated from its host; the server default when unset.
    pub isolation: Option<Isolation>,
}

impl Run {
    /// Runs `argv` with no labels and the default timeout.
    #[must_use]
    pub const fn new(argv: Vec<String>) -> Self {
        Self {
            argv,
            labels: BTreeMap::new(),
            timeout_seconds: None,
            base: None,
            isolation: None,
        }
    }

    /// Runs in a sandbox with `isolation`.
    #[must_use]
    pub const fn with_isolation(mut self, isolation: Isolation) -> Self {
        self.isolation = Some(isolation);
        self
    }

    /// Layers the directory over snapshot `base`.
    #[must_use]
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = Some(base.into());
        self
    }

    /// Sets the sandbox labels.
    #[must_use]
    pub fn with_labels(mut self, labels: BTreeMap<String, String>) -> Self {
        self.labels = labels;
        self
    }

    /// Sets the timeout.
    #[must_use]
    pub const fn with_timeout_seconds(mut self, seconds: u32) -> Self {
        self.timeout_seconds = Some(seconds);
        self
    }
}

/// A command started by [`Client::run`]: its sandbox, its job and its output.
#[derive(Debug)]
pub struct Running {
    /// The sandbox created from the directory.
    pub sandbox: SandboxResource,
    /// The job running the command.
    pub job: JobResource,
    logs: LogStream,
}

impl Running {
    /// The job's output, ending with [`crate::LogEvent::End`].
    pub fn logs(&mut self) -> &mut LogStream {
        &mut self.logs
    }
}

impl Client {
    /// Snapshots `dir` (honouring `.gitignore`) under `/workspace`, over `run`'s base if set,
    /// starts a sandbox from it and runs `run` there.
    /// The sandbox stays until stopped.
    pub async fn run(&self, dir: &Path, run: Run) -> Result<Running, Error> {
        let dir = dir.to_path_buf();
        let layer = tokio::task::spawn_blocking(move || archive::Layer::from_dir(&dir))
            .await
            .map_err(|error| Error::Protocol(error.to_string()))??;
        let digest = self.upload_blob(layer.into_bytes()).await?;
        let mut snapshot = CreateSnapshotRequest::new(vec![Layer::new(digest, MediaType::Tar)]);
        if let Some(base) = run.base {
            snapshot = snapshot.with_base(base);
        }
        let snapshot = self.create_snapshot(&snapshot).await?;
        let mut sandbox = CreateSandboxRequest::new(snapshot.id).with_labels(run.labels);
        if let Some(isolation) = run.isolation {
            sandbox = sandbox.with_isolation(isolation);
        }
        let sandbox = self.create_sandbox(&sandbox).await?;
        let mut exec = ExecRequest::new(run.argv);
        if let Some(seconds) = run.timeout_seconds {
            exec = exec.with_timeout_seconds(seconds);
        }
        let job = self.exec(&sandbox.id, &exec).await?;
        let logs = self.logs(&job.id).await?;
        Ok(Running { sandbox, job, logs })
    }
}
