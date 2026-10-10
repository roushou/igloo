//! Igloo's git endpoint: each repository's copy served over git's smart HTTP protocol at
//! `/git/<repository id>.git`, by `git http-backend`.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::header::{
    AUTHORIZATION, CONTENT_ENCODING, CONTENT_LENGTH, HeaderName, HeaderValue, WWW_AUTHENTICATE,
};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::StreamExt as _;
use igloo_core::repo::{Repo, RepoId};
use igloo_core::workspace::Workspace;
use igloo_core::{Actor, Entity as _, Resource as _};
use igloo_git::{
    BackendProcess, BackendRequest, Exchange, Git, GitError, HttpBackend, RefName, Service,
};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::process::ChildStdout;
use tokio_stream::wrappers::ReceiverStream;
use tracing::warn;

use super::WorkspaceCredentials;
use super::WorkspaceGrant;
use super::rest::DevToken;
use crate::app::{CommandBus, InstallError, PlatformBuilder, RequestContext, TaskSpawner};
use crate::platform::{BranchPushes, RepoQueries};
use crate::ports::{IdGenerator, IdGeneratorExt as _};
use crate::workspaces::WorkspaceQueries;

/// Why the git endpoint is not available.
#[derive(Debug, thiserror::Error)]
pub enum GitHttpError {
    /// The stores it reads are missing.
    #[error(transparent)]
    Install(#[from] InstallError),
    /// The hook refusing pushes to the default branch could not be written.
    #[error("preparing the git endpoint: {0}")]
    Prepare(#[from] GitError),
}

/// The git endpoint over the copies of repositories under one directory.
///
/// Invariants: every request is authenticated with the API token, or with the credential of a
/// workspace's sandbox, as a bearer token or as the password of HTTP Basic, whatever the user
/// name; a workspace credential reaches only its workspace's repository, acts as the
/// workspace's owner and stops working when the workspace is deleted or stops running that
/// sandbox; a push never updates the repository's
/// default branch (it moves only by merge); a pushed branch opens or revises its change
/// before the push is acknowledged to the client.
#[derive(Clone)]
pub struct GitHttp {
    repos: RepoQueries,
    pushes: BranchPushes,
    backend: HttpBackend,
    git: Git,
    root: PathBuf,
    auth: DevToken,
    credentials: WorkspaceCredentials,
    workspaces: WorkspaceQueries,
    ids: Arc<dyn IdGenerator>,
    spawner: TaskSpawner,
}

/// Why a request was not served.
enum Rejection {
    Unauthenticated,
    NotFound,
    NotReady,
    Unsupported,
    Failed(String),
}

#[derive(Deserialize)]
struct AdvertiseQuery {
    service: Option<String>,
}

impl GitHttp {
    /// The user git runs as; it only has to be non-empty for `git http-backend` to accept pushes.
    const USER: &'static str = "igloo";
    /// How much of git's response headers to read.
    const MAX_HEADER_BYTES: usize = 16 * 1024;
    /// How much of git's output to hold per chunk.
    const CHUNK: usize = 64 * 1024;

    /// Serves the repositories whose copies are directly under `root`, authenticating with
    /// `auth`, reading stores from `platform` and dispatching on `bus`; background work runs
    /// on `spawner`.
    pub async fn new(
        platform: &PlatformBuilder,
        bus: CommandBus,
        auth: DevToken,
        credentials: WorkspaceCredentials,
        root: PathBuf,
        spawner: TaskSpawner,
    ) -> Result<Self, GitHttpError> {
        let ports = platform.ports();
        let git = Git::isolated();
        let backend = HttpBackend::new(git.clone(), &root);
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(GitError::Io)?;
        backend.prepare().await?;
        Ok(Self {
            repos: RepoQueries::new(platform.store::<Repo>()?, Arc::clone(&ports.secrets)),
            pushes: BranchPushes::new(platform, bus)?,
            backend,
            git,
            root,
            auth,
            credentials,
            workspaces: WorkspaceQueries::new(platform.store::<Workspace>()?),
            ids: Arc::clone(&ports.ids),
            spawner,
        })
    }

    /// The routes of the endpoint.
    pub fn router(self) -> Router {
        Router::new()
            .route("/git/{repository}/info/refs", get(Self::advertise))
            .route("/git/{repository}/git-upload-pack", post(Self::upload_pack))
            .route(
                "/git/{repository}/git-receive-pack",
                post(Self::receive_pack),
            )
            .with_state(self)
    }

    async fn advertise(
        State(git): State<Self>,
        Path(repository): Path<String>,
        Query(query): Query<AdvertiseQuery>,
        headers: HeaderMap,
    ) -> Response {
        let exchange = match query.service.as_deref() {
            Some("git-upload-pack") => Exchange::Advertise(Service::UploadPack),
            Some("git-receive-pack") => Exchange::Advertise(Service::ReceivePack),
            _ => return Rejection::Unsupported.into_response(),
        };
        git.serve(exchange, &repository, &headers, Body::empty())
            .await
    }

    async fn upload_pack(
        State(git): State<Self>,
        Path(repository): Path<String>,
        headers: HeaderMap,
        body: Body,
    ) -> Response {
        git.serve(
            Exchange::Converse(Service::UploadPack),
            &repository,
            &headers,
            body,
        )
        .await
    }

    async fn receive_pack(
        State(git): State<Self>,
        Path(repository): Path<String>,
        headers: HeaderMap,
        body: Body,
    ) -> Response {
        git.serve(
            Exchange::Converse(Service::ReceivePack),
            &repository,
            &headers,
            body,
        )
        .await
    }

    async fn serve(
        &self,
        exchange: Exchange,
        repository: &str,
        headers: &HeaderMap,
        body: Body,
    ) -> Response {
        match self.start(exchange, repository, headers, body).await {
            Ok(response) => response,
            Err(rejection) => rejection.into_response(),
        }
    }

    /// Who the request with `authorization` is from, and the repository it names.
    async fn caller(
        &self,
        authorization: Option<&str>,
        repository: &str,
    ) -> Result<(Actor, RepoId), Rejection> {
        let secret = DevToken::git_secret(authorization).ok_or(Rejection::Unauthenticated)?;
        let owner = self.auth.actor_of(&secret);
        let grant = self.credentials.verify(&secret);
        if owner.is_none() && grant.is_none() {
            return Err(Rejection::Unauthenticated);
        }
        let id: RepoId = repository
            .strip_suffix(".git")
            .and_then(|id| id.parse().ok())
            .ok_or(Rejection::NotFound)?;
        let actor = match (owner, grant) {
            (Some(actor), _) => actor,
            (None, Some(grant)) => self.workspace_actor(grant, id).await?,
            (None, None) => return Err(Rejection::Unauthenticated),
        };
        Ok((actor, id))
    }

    /// The actor a workspace credential stands for at repository `repo`: the workspace's owner,
    /// while the workspace exists, still runs the credential's sandbox and works on `repo`.
    async fn workspace_actor(
        &self,
        grant: WorkspaceGrant,
        repo: RepoId,
    ) -> Result<Actor, Rejection> {
        let workspace = self
            .workspaces
            .get(grant.workspace)
            .await
            .map_err(|error| Rejection::Failed(error.to_string()))?
            .filter(|workspace| workspace.status().sandbox() == Some(grant.sandbox))
            .ok_or(Rejection::Unauthenticated)?;
        if workspace.spec().repo != repo {
            return Err(Rejection::NotFound);
        }
        Ok(Actor::Human {
            user: workspace.spec().owner,
        })
    }

    async fn start(
        &self,
        exchange: Exchange,
        repository: &str,
        headers: &HeaderMap,
        body: Body,
    ) -> Result<Response, Rejection> {
        let header = |name: HeaderName| headers.get(name).and_then(|value| value.to_str().ok());
        let (actor, id) = self.caller(header(AUTHORIZATION), repository).await?;
        let repo = self
            .repos
            .get(id)
            .await
            .map_err(|error| Rejection::Failed(error.to_string()))?
            .ok_or(Rejection::NotFound)?;
        let name = format!("{id}.git");
        let dir = self.root.join(&name);
        // A copy is served only once it holds the default branch: a clone of an empty copy would
        // start an unrelated history.
        let copy = match self.git.open(dir).await {
            Ok(copy) => copy,
            Err(GitError::NotARepository(_)) => return Err(Rejection::NotReady),
            Err(error) => return Err(error.into()),
        };
        if copy
            .resolve(&RefName::from(repo.default_branch()))
            .await?
            .is_none()
        {
            return Err(Rejection::NotReady);
        }
        if matches!(exchange, Exchange::Advertise(_)) {
            // Clones check out the default branch.
            copy.set_head(repo.default_branch()).await?;
        }

        let mut request = BackendRequest::new(exchange, &name, Self::USER)
            .map_err(|error| Rejection::Failed(error.to_string()))?;
        if let Some(protocol) = header(HeaderName::from_static("git-protocol")) {
            request = request.with_protocol(protocol);
        }
        if header(CONTENT_ENCODING) == Some("gzip") {
            request = request.gzipped();
        }
        if let Some(length) = header(CONTENT_LENGTH).and_then(|length| length.parse().ok()) {
            request = request.with_content_length(length);
        }
        let pushing = exchange == Exchange::Converse(Service::ReceivePack);
        if pushing {
            let default = RefName::from(repo.default_branch());
            let message = format!(
                "refusing to update {}: the default branch moves only by merge; \
                 use `igloo change merge <change>`",
                repo.default_branch()
            );
            request = request.refusing(vec![default], &message);
        }
        let before = if pushing {
            Some(self.pushes.branches(&repo).await.map_err(Rejection::from)?)
        } else {
            None
        };

        let mut process = self.backend.spawn(&request)?;
        let stdin = process.take_stdin();
        let stdout = process
            .take_stdout()
            .ok_or_else(|| Rejection::Failed("git has no output".to_owned()))?;
        self.spawner.spawn("git-http-input", move |_| async move {
            let Some(mut stdin) = stdin else {
                return Ok(());
            };
            let mut chunks = body.into_data_stream();
            while let Some(Ok(chunk)) = chunks.next().await {
                if stdin.write_all(&chunk).await.is_err() {
                    break;
                }
            }
            Ok(())
        });

        let mut output = BufReader::new(stdout);
        let (status, response_headers) = Self::cgi_headers(&mut output).await?;
        let (sender, receiver) = tokio::sync::mpsc::channel(4);
        let this = self.clone();
        let context = RequestContext::new(actor, self.ids.next());
        self.spawner.spawn("git-http-output", move |_| async move {
            this.relay(process, output, &sender).await;
            if let Some(before) = before {
                // The client's push ends only once its branches are proposed: the response
                // stays open until the sender drops.
                this.record(&repo, &before, &context).await;
            }
            drop(sender);
            Ok(())
        });
        let mut response = Response::new(Body::from_stream(ReceiverStream::new(receiver)));
        *response.status_mut() = status;
        *response.headers_mut() = response_headers;
        Ok(response)
    }
}

impl GitHttp {
    /// Parses the CGI headers git printed: `Status` becomes the response's status and the rest
    /// its headers.
    async fn cgi_headers(
        output: &mut BufReader<ChildStdout>,
    ) -> Result<(StatusCode, HeaderMap), Rejection> {
        let malformed = || Rejection::Failed("git printed malformed headers".to_owned());
        let mut status = StatusCode::OK;
        let mut headers = HeaderMap::new();
        let mut read = 0;
        loop {
            let mut line = Vec::new();
            let length = output
                .read_until(b'\n', &mut line)
                .await
                .map_err(|error| Rejection::Failed(error.to_string()))?;
            read += length;
            if length == 0 || read > Self::MAX_HEADER_BYTES {
                return Err(malformed());
            }
            let line = std::str::from_utf8(&line).map_err(|_| malformed())?;
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                return Ok((status, headers));
            }
            let (name, value) = line.split_once(':').ok_or_else(malformed)?;
            let value = value.trim();
            if name.eq_ignore_ascii_case("status") {
                status = value
                    .split(' ')
                    .next()
                    .and_then(|code| code.parse::<u16>().ok())
                    .and_then(|code| StatusCode::from_u16(code).ok())
                    .ok_or_else(malformed)?;
            } else {
                headers.append(
                    HeaderName::from_bytes(name.as_bytes()).map_err(|_| malformed())?,
                    HeaderValue::from_str(value).map_err(|_| malformed())?,
                );
            }
        }
    }

    /// Forwards git's output to the client until it ends, then waits for git. A client that
    /// went away does not stop git: a push in flight completes.
    async fn relay(
        &self,
        process: BackendProcess,
        mut output: BufReader<ChildStdout>,
        sender: &tokio::sync::mpsc::Sender<Result<Bytes, std::io::Error>>,
    ) {
        let mut connected = true;
        let mut chunk = vec![0; Self::CHUNK];
        loop {
            match output.read(&mut chunk).await {
                Ok(0) => break,
                Ok(read) => {
                    if connected
                        && sender
                            .send(Ok(Bytes::copy_from_slice(&chunk[..read])))
                            .await
                            .is_err()
                    {
                        connected = false;
                    }
                }
                Err(error) => {
                    warn!(%error, "reading git's output failed");
                    break;
                }
            }
        }
        if let Err(error) = process.finish().await {
            warn!(%error, "git http-backend failed");
        }
    }

    /// Opens or revises the changes of the branches that moved since `before`.
    async fn record(
        &self,
        repo: &Repo,
        before: &crate::platform::Branches,
        context: &RequestContext,
    ) {
        let recorded = async {
            let after = self.pushes.branches(repo).await?;
            self.pushes.record(repo, before, &after, context).await
        }
        .await;
        if let Err(error) = recorded {
            warn!(repo = %repo.id(), %error, "recording a push failed");
        }
    }
}

impl From<GitError> for Rejection {
    fn from(error: GitError) -> Self {
        match error {
            GitError::NotARepository(_) => Self::NotFound,
            error => Self::Failed(error.to_string()),
        }
    }
}

impl From<crate::app::AppError> for Rejection {
    fn from(error: crate::app::AppError) -> Self {
        Self::Failed(error.to_string())
    }
}

impl IntoResponse for Rejection {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                [(WWW_AUTHENTICATE, "Basic realm=\"igloo\"")],
                "authentication required: use the API token as the password\n",
            )
                .into_response(),
            Self::NotFound => (StatusCode::NOT_FOUND, "no such repository\n").into_response(),
            Self::NotReady => (
                StatusCode::SERVICE_UNAVAILABLE,
                "the repository's default branch has not been fetched yet; try again shortly\n",
            )
                .into_response(),
            Self::Unsupported => (
                StatusCode::FORBIDDEN,
                "only git's smart HTTP protocol is served\n",
            )
                .into_response(),
            Self::Failed(error) => {
                warn!(%error, "serving git failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error\n").into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path as FsPath;
    use std::process::Stdio;

    use igloo_core::change::ChangePhase;
    use igloo_core::repo::BranchName;
    use igloo_core::{Actor, Id};
    use igloo_git::testing::Fixture;
    use tokio::net::TcpListener;
    use uuid::Uuid;

    use super::*;
    use crate::app::{AppError, TaskSupervisor};
    use crate::platform::{
        ChangeModule, ChangeQueries, ForgeMirror, MergeChange, MirrorModule, RegisterRepo,
        RepoModule,
    };
    use crate::ports::{EventLog as _, Expected, Forge, Sequence};
    use crate::testing::{MemoryPlatform, MemoryStores, START, blob_urls, context, wait_until};

    const TOKEN: &str = "git-token";

    /// A server over memory adapters with one hosted repository whose forge is a local origin.
    struct Hosted {
        url: String,
        repo: Repo,
        remote: crate::ports::Remote,
        origin: Fixture,
        forge: Arc<dyn Forge>,
        changes: ChangeQueries,
        bus: CommandBus,
        mirror: ForgeMirror,
        scratch: tempfile::TempDir,
        credentials: WorkspaceCredentials,
        _supervisor: TaskSupervisor,
        stores: MemoryStores,
    }

    impl Hosted {
        async fn start() -> Self {
            let scratch = tempfile::tempdir().expect("dir");
            let origin = Fixture::new(scratch.path());
            origin.commit("base", &[("README.md", Some("igloo"))]);
            let MemoryPlatform {
                mut builder,
                stores,
            } = MemoryPlatform::new();
            builder.install(RepoModule).expect("repo module");
            builder.install(ChangeModule).expect("change module");
            builder.install(MirrorModule).expect("mirror module");
            let supervisor = TaskSupervisor::new();
            let actor = Actor::Human {
                user: Id::from_uuid(Uuid::from_u128(7)),
            };
            let credentials =
                WorkspaceCredentials::new(&blob_urls(Arc::clone(&builder.ports().clock)));
            let git = GitHttp::new(
                &builder,
                builder.bus(),
                DevToken::new(TOKEN, actor),
                credentials.clone(),
                stores.forge_dir.path().to_path_buf(),
                supervisor.spawner(),
            )
            .await
            .expect("git endpoint");
            let forge = Arc::clone(&builder.ports().forge);
            let mirror = ForgeMirror::new(&builder).expect("mirror");
            let repos = RepoQueries::new(
                builder.store::<Repo>().expect("repos"),
                Arc::clone(&builder.ports().secrets),
            );
            let changes = ChangeQueries::new(builder.store().expect("changes"));
            let bus = builder.build().start(&supervisor);

            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            let address = listener.local_addr().expect("address");
            let router = git.router();
            supervisor.spawn("test-server", move |cancel| async move {
                axum::serve(listener, router)
                    .with_graceful_shutdown(cancel.cancelled_owned())
                    .await
                    .map_err(AppError::infrastructure)
            });
            let main: BranchName = "main".parse().expect("branch");
            let command = RegisterRepo {
                location: origin.location(),
                default_branch: main.clone(),
                token: None,
            };
            let id = bus.dispatch(command, context()).await.expect("register");
            let repo = repos.get(id).await.expect("repos").expect("repo");
            let remote = repos.remote(&repo).await.expect("remote");
            // Registering a repository creates Igloo's copy of it.
            wait_until(async || {
                forge
                    .mirrored(repo.id(), &main)
                    .await
                    .expect("mirrored")
                    .is_some()
            })
            .await;
            Self {
                url: format!("{address}/git/{id}.git"),
                repo,
                remote,
                origin,
                forge,
                changes,
                bus,
                mirror,
                scratch,
                credentials,
                _supervisor: supervisor,
                stores,
            }
        }

        /// The repository's URL, with `credentials` (`user:password`) in it when given.
        fn url(&self, credentials: Option<&str>) -> String {
            credentials.map_or_else(
                || format!("http://{}", self.url),
                |credentials| format!("http://{credentials}@{}", self.url),
            )
        }

        fn authorized(&self) -> String {
            self.url(Some(&format!("anyone:{TOKEN}")))
        }

        fn branch(name: &str) -> BranchName {
            name.parse().expect("branch")
        }

        async fn copy_head(&self, branch: &str) -> Option<igloo_core::repo::CommitId> {
            self.forge
                .mirrored(self.repo.id(), &Self::branch(branch))
                .await
                .expect("mirrored")
        }

        /// The commit the forge has `branch` at, if it has it.
        async fn origin_head(&self, branch: &str) -> Option<String> {
            let reference = format!("refs/heads/{branch}");
            let (ok, said) = self
                .git(
                    self.origin.origin(),
                    &["rev-parse", "--verify", "--quiet", &reference],
                )
                .await;
            ok.then(|| said.trim().to_owned())
        }

        async fn clone(&self, into: &str) -> std::path::PathBuf {
            let work = self.scratch.path().join(into);
            let (ok, said) = self
                .git(self.scratch.path(), &["clone", &self.authorized(), into])
                .await;
            assert!(ok, "{said}");
            work
        }

        /// Runs the stock `git` in `dir`; returns whether it succeeded and what it printed.
        async fn git(&self, dir: &FsPath, args: &[&str]) -> (bool, String) {
            let output = tokio::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("LC_ALL", "C")
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@igloo.invalid")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@igloo.invalid")
                .stdin(Stdio::null())
                .output()
                .await
                .expect("git");
            let said = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            (output.status.success(), said)
        }

        async fn ok(&self, dir: &FsPath, args: &[&str]) -> String {
            let (ok, said) = self.git(dir, args).await;
            assert!(ok, "git {args:?}: {said}");
            said.trim().to_owned()
        }

        /// Writes `file`, commits it with `message` in `work` and returns the commit.
        async fn commit(&self, work: &FsPath, file: &str, message: &str) -> String {
            std::fs::write(work.join(file), message).expect("write");
            self.ok(work, &["add", "."]).await;
            self.ok(work, &["commit", "--quiet", "-m", message]).await;
            self.ok(work, &["rev-parse", "HEAD"]).await
        }

        async fn changes(&self) -> Vec<igloo_core::change::Change> {
            self.changes.of_repo(self.repo.id()).await.expect("changes")
        }
    }

    impl Hosted {
        const OWNER: u128 = 9;
        const SANDBOX: u128 = 5;

        /// A workspace of `repo` whose sandbox runs; its sandbox and a credential for it.
        async fn workspace(
            &self,
            repo: igloo_core::repo::RepoId,
        ) -> (
            igloo_core::workspace::WorkspaceId,
            igloo_core::sandbox::SandboxId,
            String,
        ) {
            let id = Id::from_uuid(Uuid::from_u128(40));
            let sandbox = Id::from_uuid(Uuid::from_u128(Self::SANDBOX));
            let owner = Id::from_uuid(Uuid::from_u128(Self::OWNER));
            let now = START;
            let mut workspace =
                igloo_core::workspace::Workspace::new(id, owner, repo, Self::branch("main"), now);
            workspace
                .sandbox_created(sandbox, std::collections::BTreeSet::new())
                .expect("sandbox");
            workspace.sandbox_running(sandbox, now);
            let mut versioned = crate::ports::Versioned::new(workspace);
            self.stores
                .workspaces
                .commit(&mut versioned, &context().commit_meta(now))
                .await
                .expect("commit");
            (id, sandbox, self.credentials.mint(id, sandbox))
        }

        /// Clones the repository as a sandbox would, with `token` in an `Authorization` header.
        async fn clone_with(&self, token: &str, into: &str) -> (bool, String) {
            let header = format!("http.extraHeader=Authorization: Bearer {token}");
            self.git(
                self.scratch.path(),
                &["-c", &header, "clone", &self.url(None), into],
            )
            .await
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_workspaces_credential_pushes_its_repositorys_branches_as_its_owner() {
        let hosted = Hosted::start().await;
        let (_, _, token) = hosted.workspace(hosted.repo.id()).await;
        assert!(!token.contains(TOKEN));
        let (ok, said) = hosted.clone_with(&token, "checkout").await;
        assert!(ok, "{said}");
        let work = hosted.scratch.path().join("checkout");

        let header = format!("http.extraHeader=Authorization: Bearer {token}");
        hosted
            .ok(&work, &["switch", "--quiet", "-c", "feature"])
            .await;
        hosted.commit(&work, "a", "Add a").await;
        hosted
            .ok(
                &work,
                &["-c", &header, "push", "--quiet", "origin", "feature"],
            )
            .await;
        let changes = hosted.changes().await;
        assert_eq!(changes.len(), 1, "a push from the workspace opens a change");

        hosted.commit(&work, "b", "Add b").await;
        let (ok, said) = hosted
            .git(&work, &["-c", &header, "push", "origin", "HEAD:main"])
            .await;
        assert!(!ok, "the default branch still moves only by merge: {said}");
        assert!(said.contains("igloo change merge"), "{said}");

        let actors = hosted
            .stores
            .log
            .read(Sequence::default(), 10_000)
            .await
            .expect("events");
        let opened = actors
            .iter()
            .find(|event| event.kind == "igloo.change.opened")
            .expect("opened");
        assert_eq!(
            opened.actor.principal(),
            Some(Id::from_uuid(Uuid::from_u128(Hosted::OWNER))),
            "acts as the workspace's owner"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_workspaces_credential_stops_at_its_repository_its_sandbox_and_its_life() {
        let hosted = Hosted::start().await;
        let (id, sandbox, token) = hosted.workspace(hosted.repo.id()).await;

        let other = hosted.scratch.path().join("other.git");
        let command = RegisterRepo {
            location: igloo_core::repo::RepoLocation::Local {
                path: other.display().to_string(),
            },
            default_branch: Hosted::branch("main"),
            token: None,
        };
        let other_id = hosted
            .bus
            .dispatch(command, context())
            .await
            .expect("register");
        let header = format!("http.extraHeader=Authorization: Bearer {token}");
        let url = hosted
            .url(None)
            .replace(&hosted.repo.id().to_string(), &other_id.to_string());
        let (ok, said) = hosted
            .git(
                hosted.scratch.path(),
                &["-c", &header, "clone", &url, "elsewhere"],
            )
            .await;
        assert!(!ok, "another repository: {said}");
        assert!(said.contains("not found"), "{said}");

        let (ok, said) = hosted.clone_with(&format!("{token}0"), "forged").await;
        assert!(!ok, "{said}");
        assert!(said.contains("could not read Username"), "{said}");

        // The sandbox ends: the workspace no longer runs it.
        let stored = hosted
            .stores
            .workspaces
            .load(id)
            .await
            .expect("load")
            .expect("workspace");
        let mut stored = stored;
        stored.entity_mut().sandbox_ended(sandbox);
        hosted
            .stores
            .workspaces
            .commit(&mut stored, &context().commit_meta(START))
            .await
            .expect("commit");
        let (ok, said) = hosted.clone_with(&token, "ended").await;
        assert!(!ok, "{said}");
        assert!(said.contains("could not read Username"), "{said}");
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn clones_and_fetches_with_the_token_and_not_without_it() {
        let hosted = Hosted::start().await;
        let scratch = hosted.scratch.path();
        for url in [
            hosted.url(None),
            hosted.url(Some(&format!("anyone:{TOKEN}x"))),
        ] {
            let (ok, said) = hosted.git(scratch, &["clone", &url, "denied"]).await;
            assert!(!ok, "cloned without the token: {said}");
        }
        let work = hosted.clone("checkout").await;
        assert_eq!(
            std::fs::read_to_string(work.join("README.md")).expect("file"),
            "igloo",
            "a clone checks out the default branch"
        );
        assert_eq!(
            hosted.ok(&work, &["rev-parse", "HEAD"]).await,
            hosted.origin.head("main").as_str(),
            "a clone holds the origin's commit"
        );
        let bearer = format!("http.extraHeader=Authorization: Bearer {TOKEN}");
        let (ok, said) = hosted
            .git(
                scratch,
                &["-c", &bearer, "clone", &hosted.url(None), "bearer"],
            )
            .await;
        assert!(ok, "{said}");

        let second = hosted.origin.commit("second", &[("second", Some("2"))]);
        let main = Hosted::branch("main");
        hosted
            .forge
            .fetch(&hosted.remote, &main)
            .await
            .expect("fetch");
        hosted.ok(&work, &["fetch", "--quiet"]).await;
        assert_eq!(
            hosted.ok(&work, &["rev-parse", "origin/main"]).await,
            second.as_str()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_copy_without_the_default_branch_is_not_served() {
        let hosted = Hosted::start().await;
        let location = igloo_core::repo::RepoLocation::Local {
            path: hosted
                .scratch
                .path()
                .join("missing.git")
                .display()
                .to_string(),
        };
        let command = RegisterRepo {
            location,
            default_branch: Hosted::branch("main"),
            token: None,
        };
        let id = hosted
            .bus
            .dispatch(command, context())
            .await
            .expect("register");
        // Hosting initializes the copy, then fails to fetch the missing origin: a half-created
        // copy that lacks the default branch.
        let copy = hosted.stores.forge_dir.path().join(format!("{id}.git"));
        let scratch = hosted.scratch.path();
        let mut initialized = false;
        for _ in 0..100 {
            if tokio::fs::try_exists(copy.join("HEAD"))
                .await
                .unwrap_or(false)
            {
                initialized = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(initialized, "hosting initializes the copy");
        let url = hosted
            .authorized()
            .replace(&hosted.repo.id().to_string(), &id.to_string());
        let (ok, said) = hosted.git(scratch, &["clone", &url, "empty"]).await;
        assert!(!ok, "cloned an empty copy: {said}");
        assert!(said.contains("503"), "{said}");
        assert!(!scratch.join("empty").exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_push_to_the_default_branch_is_refused_naming_the_merge() {
        let hosted = Hosted::start().await;
        let work = hosted.clone("checkout").await;
        let before = hosted.copy_head("main").await;
        hosted.commit(&work, "direct", "direct").await;
        let (ok, said) = hosted.git(&work, &["push", "origin", "HEAD:main"]).await;
        assert!(!ok, "{said}");
        assert!(said.contains("igloo change merge"), "{said}");
        let (ok, said) = hosted.git(&work, &["push", "origin", ":main"]).await;
        assert!(!ok, "deleting the default branch: {said}");
        let (ok, said) = hosted
            .git(&work, &["push", "origin", "HEAD:refs/igloo/imported"])
            .await;
        assert!(!ok, "Igloo's own refs are not writable: {said}");
        assert_eq!(hosted.copy_head("main").await, before);
        assert_eq!(hosted.changes().await.len(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_pushed_branch_opens_a_change_and_later_pushes_revise_it() {
        let hosted = Hosted::start().await;
        let work = hosted.clone("checkout").await;
        hosted
            .ok(&work, &["switch", "--quiet", "-c", "feature"])
            .await;
        let first = hosted.commit(&work, "a", "Add a\n\nWith a body.").await;
        hosted
            .ok(&work, &["push", "--quiet", "origin", "feature"])
            .await;
        assert_eq!(
            hosted.copy_head("feature").await.map(|c| c.to_string()),
            Some(first.clone())
        );
        let changes = hosted.changes().await;
        assert_eq!(changes.len(), 1, "the change exists when the push returns");
        let change = &changes[0];
        assert_eq!(change.title(), "Add a");
        assert_eq!(change.source().as_str(), "feature");
        assert_eq!(*change.phase(), ChangePhase::Open);
        assert_eq!(change.latest().number, 1);
        assert_eq!(change.latest().head.as_str(), first);

        let second = hosted.commit(&work, "b", "Add b").await;
        hosted
            .ok(&work, &["push", "--quiet", "origin", "feature"])
            .await;
        hosted
            .ok(&work, &["push", "--quiet", "origin", "feature"])
            .await;
        hosted
            .ok(
                &work,
                &["commit", "--quiet", "--amend", "-m", "Add b again"],
            )
            .await;
        let amended = hosted.ok(&work, &["rev-parse", "HEAD"]).await;
        hosted
            .ok(&work, &["push", "--quiet", "--force", "origin", "feature"])
            .await;
        let changes = hosted.changes().await;
        assert_eq!(
            changes.len(),
            1,
            "a branch with an open change opens no other"
        );
        let numbers: Vec<(u32, String)> = changes[0]
            .revisions()
            .map(|revision| (revision.number, revision.head.to_string()))
            .collect();
        assert_eq!(
            numbers,
            vec![(1, first), (2, second), (3, amended)],
            "one revision per head; pushing a head again records nothing"
        );

        // A branch with nothing to propose opens no change.
        hosted
            .ok(
                &work,
                &["push", "--quiet", "origin", "origin/main:refs/heads/same"],
            )
            .await;
        assert_eq!(hosted.changes().await.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn change_branches_and_merges_reach_the_forge() {
        let hosted = Hosted::start().await;
        let work = hosted.clone("checkout").await;
        hosted
            .ok(&work, &["switch", "--quiet", "-c", "feature"])
            .await;
        let first = hosted.commit(&work, "a", "Add a").await;
        hosted
            .ok(&work, &["push", "--quiet", "origin", "feature"])
            .await;
        wait_until(async || hosted.origin_head("feature").await.as_deref() == Some(first.as_str()))
            .await;
        let second = hosted.commit(&work, "b", "Add b").await;
        hosted
            .ok(&work, &["push", "--quiet", "origin", "feature"])
            .await;
        wait_until(async || {
            hosted.origin_head("feature").await.as_deref() == Some(second.as_str())
        })
        .await;

        // A merge moves the default branch of Igloo's copy; the forge follows.
        let change = hosted.changes().await.remove(0);
        let main = Hosted::branch("main");
        let onto = hosted.copy_head("main").await.expect("main");
        let squash = hosted
            .forge
            .squash(
                hosted.repo.id(),
                &onto,
                &change.latest().head,
                "Add a and b",
                START,
            )
            .await
            .expect("squash");
        hosted
            .forge
            .advance(hosted.repo.id(), &main, &squash, Expected::At(onto))
            .await
            .expect("advance");
        assert_ne!(hosted.origin.head("main"), squash);
        let merge = MergeChange {
            change: igloo_core::Entity::id(&change),
            revision: change.latest().number,
            commit: squash.clone(),
        };
        hosted.bus.dispatch(merge, context()).await.expect("merge");
        wait_until(async || hosted.origin.head("main") == squash).await;
        wait_until(async || {
            hosted.origin_head("feature").await.is_none()
                && hosted.copy_head("feature").await.is_none()
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn the_sweep_pushes_the_default_branch_but_never_over_a_forge_that_moved() {
        let hosted = Hosted::start().await;
        let main = Hosted::branch("main");
        let onto = hosted.copy_head("main").await.expect("main");
        let work = hosted.clone("checkout").await;
        let head = hosted.commit(&work, "a", "Add a").await;
        hosted
            .ok(&work, &["switch", "--quiet", "-c", "feature"])
            .await;
        hosted
            .ok(&work, &["push", "--quiet", "origin", "feature"])
            .await;
        let head: igloo_core::repo::CommitId = head.parse().expect("commit");
        hosted
            .forge
            .advance(hosted.repo.id(), &main, &head, Expected::At(onto.clone()))
            .await
            .expect("advance");
        hosted.mirror.sweep().await.expect("sweep");
        assert_eq!(hosted.origin.head("main"), head);

        // The forge moved on its own: the copy is not forced over it.
        let elsewhere = hosted
            .origin
            .commit("elsewhere", &[("elsewhere", Some("x"))]);
        hosted.mirror.sweep().await.expect("sweep");
        assert_eq!(hosted.origin.head("main"), elsewhere);
    }
}
