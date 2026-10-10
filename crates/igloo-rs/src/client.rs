use igloo_api::change::{ApproveRequest, ChangeResource, CommentRequest, OpenChangeRequest};
use igloo_api::job::{ExecRequest, JobResource};
use igloo_api::problem::Problem;
use igloo_api::repo::{
    DotfilesResource, RegisterRepoRequest, RepoResource, RepoSnapshotRequest, RepoSnapshotResource,
    SecretList,
};
use igloo_api::run::RunResource;
use igloo_api::sandbox::{CreateSandboxRequest, SandboxList, SandboxResource};
use igloo_api::seal::{SealPhase, SealResource};
use igloo_api::snapshot::{CreateSnapshotRequest, ImportImageRequest, SnapshotResource};
use igloo_api::task::{CreateTaskRequest, TaskResource, TranscriptResource};
use igloo_api::workspace::{CreateWorkspaceRequest, WorkspaceResource};
use reqwest::{Method, RequestBuilder, Url};
use serde::de::DeserializeOwned;

use crate::logs::LogStream;
use crate::terminal::Terminal;

/// A client of the Igloo REST API.
#[derive(Clone, Debug)]
pub struct Client {
    http: reqwest::Client,
    base: Url,
    token: String,
}

/// Why a call failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The server refused the request; `code` in the problem says why.
    #[error("{}: {}", .0.code, .0.title)]
    Api(Box<Problem>),
    /// The request could not be sent or the response read.
    #[error("HTTP failure")]
    Http(#[from] reqwest::Error),
    /// The response did not match the API contract.
    #[error("unexpected response: {0}")]
    Protocol(String),
    /// A seal ended without a snapshot; carries the seal's failure reason.
    #[error("the seal failed: {0}")]
    SealFailed(String),
    /// A local file could not be read.
    #[error("local I/O failure")]
    Io(#[from] std::io::Error),
    /// A workspace could not be brought to the state asked for; the message says why.
    #[error("{0}")]
    Workspace(String),
}

impl Client {
    /// A client for the API at `base` (such as `http://127.0.0.1:7000`) using bearer `token`.
    /// Installs `ring` as the process's TLS crypto provider unless one is installed already.
    pub fn new(base: &str, token: impl Into<String>) -> Result<Self, Error> {
        let mut base = Url::parse(base).map_err(|error| Error::Protocol(error.to_string()))?;
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        // Fails only when a provider is already installed, which serves equally well.
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
        Ok(Self {
            http: reqwest::Client::builder().build()?,
            base,
            token: token.into(),
        })
    }

    /// Uploads `bytes` as a blob; returns its digest. Uploading the same bytes again is cheap.
    pub async fn upload_blob(&self, bytes: Vec<u8>) -> Result<String, Error> {
        let digest = format!("blake3:{}", blake3::hash(&bytes).to_hex());
        let request = self
            .request(Method::PUT, &format!("v1/blobs/{digest}"))?
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(bytes);
        self.send_empty(request).await?;
        Ok(digest)
    }

    /// Registers a snapshot from uploaded layer blobs, over a base snapshot's layers if the
    /// request names one.
    pub async fn create_snapshot(
        &self,
        request: &CreateSnapshotRequest,
    ) -> Result<SnapshotResource, Error> {
        let request = Self::json(self.request(Method::POST, "v1/snapshots")?, request)?;
        self.send(request).await
    }

    /// Has the server pull a public container image and register it as a snapshot.
    pub async fn import_image(
        &self,
        request: &ImportImageRequest,
    ) -> Result<SnapshotResource, Error> {
        let request = Self::json(self.request(Method::POST, "v1/snapshots/import")?, request)?;
        self.send(request).await
    }

    /// Creates a sandbox.
    pub async fn create_sandbox(
        &self,
        request: &CreateSandboxRequest,
    ) -> Result<SandboxResource, Error> {
        let request = Self::json(self.request(Method::POST, "v1/sandboxes")?, request)?;
        self.send(request).await
    }

    /// Gets a sandbox.
    pub async fn get_sandbox(&self, id: &str) -> Result<SandboxResource, Error> {
        self.send(self.request(Method::GET, &format!("v1/sandboxes/{id}"))?)
            .await
    }

    /// Lists sandboxes carrying every `key=value` label, a page at a time.
    pub async fn list_sandboxes(
        &self,
        labels: &[(String, String)],
        cursor: Option<&str>,
    ) -> Result<SandboxList, Error> {
        let mut url = self.url("v1/sandboxes")?;
        {
            let mut query = url.query_pairs_mut();
            for (key, value) in labels {
                query.append_pair("label", &format!("{key}={value}"));
            }
            if let Some(cursor) = cursor {
                query.append_pair("cursor", cursor);
            }
        }
        self.send(self.http.get(url).bearer_auth(&self.token)).await
    }

    /// Stops a sandbox. Idempotent.
    pub async fn stop_sandbox(&self, id: &str) -> Result<SandboxResource, Error> {
        self.send(self.request(Method::POST, &format!("v1/sandboxes/{id}/stop"))?)
            .await
    }

    /// Requests a seal of a running sandbox; it completes in the background.
    pub async fn create_seal(&self, sandbox: &str) -> Result<SealResource, Error> {
        self.send(self.request(Method::POST, &format!("v1/sandboxes/{sandbox}/snapshot"))?)
            .await
    }

    /// Gets a seal.
    pub async fn get_seal(&self, id: &str) -> Result<SealResource, Error> {
        self.send(self.request(Method::GET, &format!("v1/seals/{id}"))?)
            .await
    }

    /// Seals a running sandbox and waits for the snapshot; returns its id.
    pub async fn seal(&self, sandbox: &str) -> Result<String, Error> {
        let mut seal = self.create_seal(sandbox).await?;
        let mut delay = std::time::Duration::from_millis(100);
        loop {
            match seal.phase {
                SealPhase::Pending => {}
                SealPhase::Sealed => {
                    return seal
                        .snapshot
                        .ok_or_else(|| Error::Protocol("a sealed seal has no snapshot".into()));
                }
                _ => {
                    return Err(Error::SealFailed(
                        seal.failure_reason.unwrap_or_else(|| "unknown".into()),
                    ));
                }
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(std::time::Duration::from_secs(2));
            seal = self.get_seal(&seal.id).await?;
        }
    }

    /// Registers a repository.
    pub async fn register_repo(
        &self,
        request: &RegisterRepoRequest,
    ) -> Result<RepoResource, Error> {
        let request = Self::json(self.request(Method::POST, "v1/repos")?, request)?;
        self.send(request).await
    }

    /// Lists repositories, oldest first.
    pub async fn list_repos(&self) -> Result<Vec<RepoResource>, Error> {
        self.send(self.request(Method::GET, "v1/repos")?).await
    }

    /// Snapshots a repository at a branch's head or a commit.
    pub async fn snapshot_repo(
        &self,
        repo: &str,
        request: &RepoSnapshotRequest,
    ) -> Result<RepoSnapshotResource, Error> {
        let path = format!("v1/repos/{repo}/snapshots");
        let request = Self::json(self.request(Method::POST, &path)?, request)?;
        self.send(request).await
    }

    /// Opens a change proposing a branch pushed to the forge.
    pub async fn open_change(
        &self,
        repo: &str,
        request: &OpenChangeRequest,
    ) -> Result<ChangeResource, Error> {
        let path = format!("v1/repos/{repo}/changes");
        let request = Self::json(self.request(Method::POST, &path)?, request)?;
        self.send(request).await
    }

    /// A repository's changes, oldest first.
    pub async fn list_changes(&self, repo: &str) -> Result<Vec<ChangeResource>, Error> {
        self.send(self.request(Method::GET, &format!("v1/repos/{repo}/changes"))?)
            .await
    }

    /// Gets a change.
    pub async fn get_change(&self, id: &str) -> Result<ChangeResource, Error> {
        self.send(self.request(Method::GET, &format!("v1/changes/{id}"))?)
            .await
    }

    /// Records the change's branch head as its next revision, if it moved.
    pub async fn revise_change(&self, id: &str) -> Result<ChangeResource, Error> {
        self.send(self.request(Method::POST, &format!("v1/changes/{id}/revisions"))?)
            .await
    }

    /// The runs of a change, one per revision, oldest first.
    pub async fn list_runs(&self, change: &str) -> Result<Vec<RunResource>, Error> {
        self.send(self.request(Method::GET, &format!("v1/changes/{change}/runs"))?)
            .await
    }

    /// Gets a run.
    pub async fn get_run(&self, id: &str) -> Result<RunResource, Error> {
        self.send(self.request(Method::GET, &format!("v1/runs/{id}"))?)
            .await
    }

    /// Approves a change's latest revision.
    pub async fn approve_change(&self, id: &str) -> Result<ChangeResource, Error> {
        let path = format!("v1/changes/{id}/approve");
        let request = Self::json(
            self.request(Method::POST, &path)?,
            &ApproveRequest::default(),
        )?;
        self.send(request).await
    }

    /// Merges a change: Igloo pushes its latest revision to the target branch.
    pub async fn merge_change(&self, id: &str) -> Result<ChangeResource, Error> {
        self.send(self.request(Method::POST, &format!("v1/changes/{id}/merge"))?)
            .await
    }

    /// Comments on a change.
    pub async fn comment_change(
        &self,
        id: &str,
        request: &CommentRequest,
    ) -> Result<ChangeResource, Error> {
        let path = format!("v1/changes/{id}/comments");
        let request = Self::json(self.request(Method::POST, &path)?, request)?;
        self.send(request).await
    }

    /// Asks for another revision, carrying the comments since the previous request.
    pub async fn request_changes(&self, id: &str) -> Result<ChangeResource, Error> {
        self.send(self.request(Method::POST, &format!("v1/changes/{id}/request-changes"))?)
            .await
    }

    /// Closes a change without merging.
    pub async fn close_change(&self, id: &str) -> Result<ChangeResource, Error> {
        self.send(self.request(Method::POST, &format!("v1/changes/{id}/close"))?)
            .await
    }

    /// Creates a task: an agent works toward `request`'s goal from the head of the
    /// repository's default branch.
    pub async fn create_task(
        &self,
        repo: &str,
        request: &CreateTaskRequest,
    ) -> Result<TaskResource, Error> {
        let path = format!("v1/repos/{repo}/tasks");
        let request = Self::json(self.request(Method::POST, &path)?, request)?;
        self.send(request).await
    }

    /// A repository's tasks, oldest first.
    pub async fn list_tasks(&self, repo: &str) -> Result<Vec<TaskResource>, Error> {
        self.send(self.request(Method::GET, &format!("v1/repos/{repo}/tasks"))?)
            .await
    }

    /// Gets a task.
    pub async fn get_task(&self, id: &str) -> Result<TaskResource, Error> {
        self.send(self.request(Method::GET, &format!("v1/tasks/{id}"))?)
            .await
    }

    /// A task's transcript from position `after` on; read on from the response's `next`.
    pub async fn get_transcript(&self, id: &str, after: u32) -> Result<TranscriptResource, Error> {
        let path = format!("v1/tasks/{id}/transcript?after={after}");
        self.send(self.request(Method::GET, &path)?).await
    }

    /// Cancels a task and stops its sandbox.
    pub async fn cancel_task(&self, id: &str) -> Result<TaskResource, Error> {
        self.send(self.request(Method::POST, &format!("v1/tasks/{id}/cancel"))?)
            .await
    }

    /// Opens a workspace on a branch of a repository; it starts at once.
    pub async fn create_workspace(
        &self,
        repo: &str,
        request: &CreateWorkspaceRequest,
    ) -> Result<WorkspaceResource, Error> {
        let path = format!("v1/repos/{repo}/workspaces");
        let request = Self::json(self.request(Method::POST, &path)?, request)?;
        self.send(request).await
    }

    /// The caller's workspaces, oldest first.
    pub async fn list_workspaces(&self) -> Result<Vec<WorkspaceResource>, Error> {
        self.send(self.request(Method::GET, "v1/workspaces")?).await
    }

    /// Gets a workspace.
    pub async fn get_workspace(&self, id: &str) -> Result<WorkspaceResource, Error> {
        self.send(self.request(Method::GET, &format!("v1/workspaces/{id}"))?)
            .await
    }

    /// Starts a workspace, resuming from what its last stop sealed.
    pub async fn start_workspace(&self, id: &str) -> Result<WorkspaceResource, Error> {
        self.send(self.request(Method::POST, &format!("v1/workspaces/{id}/start"))?)
            .await
    }

    /// Stops a workspace: its changes are sealed, then its sandbox stops.
    pub async fn stop_workspace(&self, id: &str) -> Result<WorkspaceResource, Error> {
        self.send(self.request(Method::POST, &format!("v1/workspaces/{id}/stop"))?)
            .await
    }

    /// Deletes a workspace, dropping its unsealed changes.
    pub async fn delete_workspace(&self, id: &str) -> Result<(), Error> {
        self.send_empty(self.request(Method::DELETE, &format!("v1/workspaces/{id}"))?)
            .await
    }

    /// Opens a terminal in a running sandbox, with a screen of `cols` by `rows` characters.
    /// `command` is the program and its arguments; empty runs the sandbox's default shell. A
    /// workspace's terminal is its sandbox's. The process ends when the terminal is dropped.
    pub async fn open_terminal(
        &self,
        sandbox: &str,
        command: &[String],
        cols: u16,
        rows: u16,
    ) -> Result<Terminal, Error> {
        let url = self.url(&format!("v1/sandboxes/{sandbox}/terminal"))?;
        Terminal::connect(url, &self.token, command, cols, rows).await
    }

    /// Sets the dotfiles new workspaces of a repository are set up with.
    pub async fn set_dotfiles(
        &self,
        repo: &str,
        dotfiles: &DotfilesResource,
    ) -> Result<RepoResource, Error> {
        let request = self.request(Method::PUT, &format!("v1/repos/{repo}/dotfiles"))?;
        self.send(Self::json(request, dotfiles)?).await
    }

    /// Clears a repository's dotfiles.
    pub async fn clear_dotfiles(&self, repo: &str) -> Result<(), Error> {
        self.send_empty(self.request(Method::DELETE, &format!("v1/repos/{repo}/dotfiles"))?)
            .await
    }

    /// Sets or replaces a repository secret.
    pub async fn set_secret(&self, repo: &str, name: &str, value: &str) -> Result<(), Error> {
        let request = self
            .request(Method::PUT, &format!("v1/repos/{repo}/secrets/{name}"))?
            .header(reqwest::header::CONTENT_TYPE, "text/plain")
            .body(value.to_owned());
        self.send_empty(request).await
    }

    /// The names of a repository's secrets.
    pub async fn list_secrets(&self, repo: &str) -> Result<SecretList, Error> {
        self.send(self.request(Method::GET, &format!("v1/repos/{repo}/secrets"))?)
            .await
    }

    /// Deletes a repository secret.
    pub async fn delete_secret(&self, repo: &str, name: &str) -> Result<(), Error> {
        self.send_empty(self.request(Method::DELETE, &format!("v1/repos/{repo}/secrets/{name}"))?)
            .await
    }

    /// Runs a process in a sandbox as a job.
    pub async fn exec(&self, sandbox: &str, request: &ExecRequest) -> Result<JobResource, Error> {
        let path = format!("v1/sandboxes/{sandbox}/exec");
        let request = Self::json(self.request(Method::POST, &path)?, request)?;
        self.send(request).await
    }

    /// Gets a job.
    pub async fn get_job(&self, id: &str) -> Result<JobResource, Error> {
        self.send(self.request(Method::GET, &format!("v1/jobs/{id}"))?)
            .await
    }

    /// Follows a job's output until it ends.
    pub async fn logs(&self, job: &str) -> Result<LogStream, Error> {
        let request = self
            .request(Method::GET, &format!("v1/jobs/{job}/logs"))?
            .header(reqwest::header::ACCEPT, "text/event-stream");
        let response = Self::checked(request.send().await?).await?;
        Ok(LogStream::new(response))
    }

    fn url(&self, path: &str) -> Result<Url, Error> {
        self.base
            .join(path)
            .map_err(|error| Error::Protocol(error.to_string()))
    }

    fn request(&self, method: Method, path: &str) -> Result<RequestBuilder, Error> {
        Ok(self
            .http
            .request(method, self.url(path)?)
            .bearer_auth(&self.token))
    }

    fn json(
        request: RequestBuilder,
        body: &impl serde::Serialize,
    ) -> Result<RequestBuilder, Error> {
        let bytes = serde_json::to_vec(body).map_err(|error| Error::Protocol(error.to_string()))?;
        Ok(request
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(bytes))
    }

    async fn send<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T, Error> {
        let response = Self::checked(request.send().await?).await?;
        let bytes = response.bytes().await?;
        serde_json::from_slice(&bytes).map_err(|error| Error::Protocol(error.to_string()))
    }

    async fn send_empty(&self, request: RequestBuilder) -> Result<(), Error> {
        Self::checked(request.send().await?).await.map(drop)
    }

    /// The response if successful, otherwise its problem as an error.
    async fn checked(response: reqwest::Response) -> Result<reqwest::Response, Error> {
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status();
        let bytes = response.bytes().await?;
        let problem = serde_json::from_slice::<Problem>(&bytes)
            .map_err(|_| Error::Protocol(format!("{status} without a problem body")))?;
        Err(Error::Api(Box::new(problem)))
    }
}
