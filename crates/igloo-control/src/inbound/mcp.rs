//! The MCP server at `/mcp` (streamable HTTP): tools over the same commands and resources as
//! the REST API.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use igloo_api::change::CommentRequest;
use igloo_api::task::CreateTaskRequest;
use igloo_core::Actor;
use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig};
use rmcp::schemars::{self, JsonSchema};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};

use super::rest::{ApiError, ApiState, RestApi, changes, repos, runs, tasks};
use crate::app::RequestContext;
use crate::ports::IdGeneratorExt;

/// Igloo's MCP server: tools over the same commands and resources as the REST API, returning
/// the same resources and problems, authenticated by the same bearer token. Tools neither
/// approve nor merge.
#[derive(Clone)]
pub struct Mcp {
    state: ApiState,
}

/// A repository.
#[derive(Deserialize, JsonSchema)]
struct RepoParams {
    /// The repository id (`repo_...`), from `repo.list`.
    repo: String,
}

/// A task to create.
#[derive(Deserialize, JsonSchema)]
struct CreateTaskParams {
    /// The repository id (`repo_...`), from `repo.list`.
    repo: String,
    /// What the agent is asked to do; it starts from the default branch's head.
    goal: String,
    /// A tool of the repository's `.igloo/agents.toml`; its default tool when absent.
    tool: Option<String>,
}

/// An object by id.
#[derive(Deserialize, JsonSchema)]
struct IdParams {
    /// The id.
    id: String,
}

/// Where a task's transcript is read from.
#[derive(Deserialize, JsonSchema)]
struct TranscriptParams {
    /// The task id (`task_...`).
    id: String,
    /// The first position to return; the `next` of the previous read, 0 when absent.
    after: Option<u32>,
}

/// A comment on a change.
#[derive(Deserialize, JsonSchema)]
struct CommentParams {
    /// The change id (`chg_...`).
    id: String,
    /// What the comment says.
    body: String,
    /// The revision; the latest when absent.
    revision: Option<u32>,
    /// The file it is about, relative to the repository root.
    path: Option<String>,
    /// The line of `path` it is about, from 1.
    line: Option<u32>,
}

/// A change with the runs of its checks.
#[derive(Serialize)]
struct ChangeWithRuns {
    change: igloo_api::change::ChangeResource,
    runs: Vec<igloo_api::run::RunResource>,
}

#[tool_router]
impl Mcp {
    /// Igloo's tools over `state`.
    const fn new(state: ApiState) -> Self {
        Self { state }
    }

    /// The MCP service over `rest`'s state, refusing requests without the bearer token.
    pub fn router(rest: &RestApi) -> Router {
        let state = rest.state().clone();
        let config = StreamableHttpServerConfig::default().disable_allowed_hosts();
        let tools = state.clone();
        let service = StreamableHttpService::new(
            move || Ok(Self::new(tools.clone())),
            Arc::new(LocalSessionManager::default()),
            config,
        );
        Router::new()
            .nest_service(Self::PATH, service)
            .layer(axum::middleware::from_fn_with_state(
                state,
                Self::authenticate,
            ))
    }

    #[tool(
        name = "repo.list",
        description = "Lists the repositories registered on Igloo."
    )]
    async fn repo_list(&self) -> CallToolResult {
        Self::result(repos::list_repos(&self.state).await)
    }

    #[tool(
        name = "task.create",
        description = "Creates a task: an agent works toward the goal in its own sandbox, from \
                       the head of the repository's default branch, and its commits become a \
                       change whose checks run. Returns the task; follow it with task.get and \
                       task.transcript."
    )]
    async fn task_create(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(params): Parameters<CreateTaskParams>,
    ) -> CallToolResult {
        let mut request = CreateTaskRequest::new(params.goal);
        if let Some(tool) = params.tool {
            request = request.with_tool(tool);
        }
        let context = self.context(&parts);
        Self::result(tasks::create_task(&self.state, context, &params.repo, request).await)
    }

    #[tool(
        name = "task.get",
        description = "Gets a task: its phase, turns, change and why it failed, if it did."
    )]
    async fn task_get(&self, Parameters(params): Parameters<IdParams>) -> CallToolResult {
        Self::result(tasks::get_task(&self.state, &params.id).await)
    }

    #[tool(
        name = "task.list",
        description = "Lists a repository's tasks, oldest first."
    )]
    async fn task_list(&self, Parameters(params): Parameters<RepoParams>) -> CallToolResult {
        Self::result(tasks::list_tasks(&self.state, &params.repo).await)
    }

    #[tool(
        name = "task.transcript",
        description = "Reads a task's transcript from a position on: what its agent said and \
                       did. Read again from `next`; `idle` means no turn runs."
    )]
    async fn task_transcript(
        &self,
        Parameters(params): Parameters<TranscriptParams>,
    ) -> CallToolResult {
        let after = params.after.unwrap_or(0);
        Self::result(tasks::read_transcript(&self.state, &params.id, after).await)
    }

    #[tool(
        name = "task.cancel",
        description = "Cancels a task and stops its sandbox; for a task that failed collecting its commits, stops the sandbox it kept."
    )]
    async fn task_cancel(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(params): Parameters<IdParams>,
    ) -> CallToolResult {
        let context = self.context(&parts);
        Self::result(tasks::cancel_task(&self.state, context, &params.id).await)
    }

    #[tool(
        name = "change.get",
        description = "Gets a change with its revisions, comments, approvals and the runs of \
                       its checks."
    )]
    async fn change_get(&self, Parameters(params): Parameters<IdParams>) -> CallToolResult {
        let found = async {
            Ok(ChangeWithRuns {
                change: changes::get_change(&self.state, &params.id).await?,
                runs: runs::runs_of_change(&self.state, &params.id).await?,
            })
        };
        Self::result(found.await)
    }

    #[tool(
        name = "change.comment",
        description = "Comments on a change's revision, the latest by default, optionally on a \
                       line of a file. Comments reach a task's agent with change.request_changes."
    )]
    async fn change_comment(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(params): Parameters<CommentParams>,
    ) -> CallToolResult {
        let mut request = CommentRequest::new(params.body);
        if let Some(revision) = params.revision {
            request = request.on_revision(revision);
        }
        if let Some(path) = params.path {
            request = request.on_path(path, params.line);
        }
        let context = self.context(&parts);
        Self::result(changes::add_comment(&self.state, context, &params.id, request).await)
    }

    #[tool(
        name = "change.request_changes",
        description = "Asks for another revision, carrying the comments since the previous \
                       request; a task's agent gets them as its next turn."
    )]
    async fn change_request_changes(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(params): Parameters<IdParams>,
    ) -> CallToolResult {
        let context = self.context(&parts);
        Self::result(changes::ask_for_changes(&self.state, context, &params.id).await)
    }
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "rmcp's tool_handler generates async methods that need not await"
)]
#[tool_handler]
impl ServerHandler for Mcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("igloo", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Igloo runs coding agents on tasks in sandboxes and checks their changes. Create \
                 tasks with task.create, follow them with task.get and task.transcript, and \
                 review their changes with change.get, change.comment and \
                 change.request_changes. A human approves and merges with the igloo CLI.",
            )
    }
}

impl Mcp {
    /// Where the service is mounted.
    pub const PATH: &'static str = "/mcp";

    /// Lets requests through when they carry the bearer token, recording the caller.
    async fn authenticate(
        State(state): State<ApiState>,
        mut request: Request,
        next: Next,
    ) -> Response {
        let header = request
            .headers()
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok());
        match state.auth.authenticate(header) {
            Some(actor) => {
                request.extensions_mut().insert(actor);
                next.run(request).await
            }
            None => ApiError::unauthenticated().into_response(),
        }
    }

    /// The command context of the authenticated caller of `parts`.
    fn context(&self, parts: &Parts) -> RequestContext {
        let actor = parts
            .extensions
            .get::<Actor>()
            .copied()
            .unwrap_or(Actor::System {
                component: igloo_core::SystemComponent::Gateway,
            });
        RequestContext::new(actor, self.state.ids.next())
    }

    /// A tool's result: the resource as JSON, or the problem as an error result.
    fn result<T: Serialize>(result: Result<T, ApiError>) -> CallToolResult {
        match result {
            Ok(value) => CallToolResult::structured(
                serde_json::to_value(value).unwrap_or(serde_json::Value::Null),
            ),
            Err(ApiError(problem)) => CallToolResult::structured_error(
                serde_json::to_value(*problem).unwrap_or(serde_json::Value::Null),
            ),
        }
    }
}
