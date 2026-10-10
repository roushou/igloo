use std::collections::BTreeMap;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::Response;
use igloo_api::problem::Problem;
use igloo_api::terminal::{
    TerminalClientFrame, TerminalMode, TerminalRequest, TerminalServerFrame,
};
use igloo_core::sandbox::SandboxPhase;
use igloo_core::terminal::TerminalSize;
use igloo_core::workspace::WorkspaceId;
use igloo_core::{Actor, Resource as _};
use tokio::time::{Interval, MissedTickBehavior};

use super::auth::TerminalCaller;
use super::sandboxes::{load, parse_id};
use super::{ApiError, ApiState};
use crate::agents::{TaskId, TaskQueries};
use crate::app::{AppError, CommandBus, RequestContext};
use crate::inbound::{TerminalEvent, TerminalLaunch, TerminalSession};
use crate::ports::IdGeneratorExt as _;
use crate::workspaces::TouchWorkspace;

/// Opens an interactive terminal in a running sandbox: upgrades to a WebSocket speaking the
/// subprotocol `igloo.terminal.v1`.
///
/// With `mode=read_only` the caller watches: the server runs its own view of the sandbox (a
/// command of the caller's is refused) and drops every input frame, so nothing the caller sends
/// reaches the sandbox. The default `read_write` forwards input. In the sandbox of a task it is
/// refused (409) unless the caller is the person who took the task over, and it stops forwarding
/// input once the task is handed back.
///
/// Binary frames carry the terminal's bytes in both directions. Text frames carry JSON control
/// messages: the client sends `TerminalClientFrame` (resize); the server sends one
/// `TerminalServerFrame` (exit) and closes the socket when the process ends. Closing the socket
/// kills the process. Authenticated like the rest of the API; a browser, which cannot set headers
/// on a WebSocket, offers the subprotocol `igloo.bearer.<token>` as well.
#[utoipa::path(
    get,
    operation_id = "openTerminal",
    path = "/v1/sandboxes/{id}/terminal",
    tag = "sandboxes",
    params(
        ("id" = String, Path),
        ("command" = Option<Vec<String>>, Query, description = "The program and its arguments, one value each; the default shell when omitted"),
        ("cols" = Option<u32>, Query, description = "Initial columns, 1 to 65535; 80 when omitted"),
        ("rows" = Option<u32>, Query, description = "Initial rows, 1 to 65535; 24 when omitted"),
        ("mode" = Option<TerminalMode>, Query, description = "`read_only` to watch, which takes no command; `read_write` (the default) to type"),
    ),
    responses(
        (status = 101, description = "Switching protocols to the WebSocket subprotocol `igloo.terminal.v1`"),
        (status = 401, body = Problem),
        (status = 404, body = Problem),
        (status = 409, body = Problem),
        (status = 422, body = Problem),
    )
)]
pub(super) async fn open(
    State(state): State<ApiState>,
    TerminalCaller(context): TerminalCaller,
    Path(id): Path<String>,
    Query(params): Query<Vec<(String, String)>>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let request = TerminalRequest::try_from(params).map_err(AppError::from)?;
    let sandbox = load(&state, parse_id(&id)?).await?;
    let status = sandbox.status();
    let (SandboxPhase::Running, Some(worker)) = (status.phase(), status.worker()) else {
        return Err(ApiError(Box::new(
            Problem::new(409, "sandbox.not_running", "The sandbox is not running")
                .with_detail(format!("the sandbox is {}", status.phase().name())),
        )));
    };
    let sandbox_id = igloo_core::Entity::id(&sandbox);
    let mut bridge = Access::default();
    let launch = if request.mode == TerminalMode::ReadWrite {
        let task = state
            .tasks
            .with_sandbox(sandbox_id)
            .await
            .map_err(AppError::from)?;
        if let Some(task) = &task {
            task.writable_by(context.actor)
                .map_err(|error| AppError::domain(&error))?;
            bridge.guard = Some(TaskGuard::new(
                state.tasks.clone(),
                igloo_core::Entity::id(task),
                context.actor,
            ));
        }
        let workspace = state.workspace_secrets.workspace_of(sandbox_id).await?;
        let mut env = state.workspace_secrets.terminal_env(sandbox_id).await?;
        if let Some(workspace) = workspace {
            env.extend(state.workspace_credentials.git_env(workspace, sandbox_id));
        }
        bridge.writable = true;
        bridge.presence =
            workspace.map(|workspace| Presence::new(state.bus.clone(), workspace, context));
        TerminalLaunch {
            sandbox: sandbox_id,
            argv: request.command,
            env,
            size: request.size,
        }
    } else {
        TerminalLaunch {
            sandbox: sandbox_id,
            argv: ReadOnlyView::argv(),
            env: BTreeMap::new(),
            size: request.size,
        }
    };
    let session = state
        .terminals
        .open(state.ids.next(), worker, launch)
        .await?;
    Ok(upgrade
        .protocols([TerminalRequest::PROTOCOL])
        .max_message_size(TerminalBridge::MAX_FRAME)
        .on_upgrade(move |socket| {
            TerminalBridge {
                socket,
                session,
                access: bridge,
            }
            .run()
        }))
}

/// The view a read-only terminal runs: the working tree's status and the latest commits of the
/// sandbox's checkout, redrawn every two seconds. It takes no git locks, so it never disturbs
/// the work it shows, and nothing the caller sends reaches it.
struct ReadOnlyView;

impl ReadOnlyView {
    const SCRIPT: &'static str = "while :; do\n\
        printf '\\033[H\\033[2J'\n\
        git --no-optional-locks status --short --branch 2>&1\n\
        printf '\\n'\n\
        git --no-optional-locks --no-pager log --oneline --decorate -n 15 2>&1\n\
        sleep 2\n\
        done\n";

    fn argv() -> Vec<String> {
        vec!["sh".to_owned(), "-c".to_owned(), Self::SCRIPT.to_owned()]
    }
}

/// What a caller's terminal may do: type, and for the sandbox of a task, only while the person
/// who took the task over holds it.
#[derive(Default)]
struct Access {
    writable: bool,
    presence: Option<Presence>,
    guard: Option<TaskGuard>,
}

/// Rechecks every [`TaskGuard::EVERY`] that the caller still holds the task whose sandbox they
/// type in, so input stops soon after the task is handed back.
struct TaskGuard {
    tasks: TaskQueries,
    task: TaskId,
    actor: Actor,
    ticker: Interval,
}

impl TaskGuard {
    const EVERY: Duration = Duration::from_secs(1);

    fn new(tasks: TaskQueries, task: TaskId, actor: Actor) -> Self {
        let mut ticker = tokio::time::interval(Self::EVERY);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Self {
            tasks,
            task,
            actor,
            ticker,
        }
    }

    /// Completes when the next check is due; the first is due at once.
    async fn due(&mut self) {
        self.ticker.tick().await;
    }

    /// Whether the caller still may type; false when the task cannot be read.
    async fn holds(&self) -> bool {
        match self.tasks.get(self.task).await {
            Ok(Some(task)) => task.writable_by(self.actor).is_ok(),
            _ => false,
        }
    }
}

/// Carries one terminal over one WebSocket until either ends.
struct TerminalBridge {
    socket: WebSocket,
    session: TerminalSession,
    access: Access,
}

/// Tells a workspace its terminal is attached, so it is not stopped for idleness: on attaching,
/// every [`Presence::EVERY`] while attached, and on detaching.
struct Presence {
    bus: CommandBus,
    workspace: WorkspaceId,
    context: RequestContext,
    ticker: Interval,
}

impl Presence {
    /// Well inside the workspace idle timeout, so one missed touch never stops a used workspace.
    const EVERY: std::time::Duration = std::time::Duration::from_mins(10);

    fn new(bus: CommandBus, workspace: WorkspaceId, context: RequestContext) -> Self {
        let mut ticker = tokio::time::interval(Self::EVERY);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Self {
            bus,
            workspace,
            context,
            ticker,
        }
    }

    /// Completes when the next touch is due; the first is due at once.
    async fn due(&mut self) {
        self.ticker.tick().await;
    }

    async fn touch(&self) {
        let command = TouchWorkspace {
            workspace: self.workspace,
        };
        if let Err(error) = self.bus.dispatch(command, self.context.clone()).await {
            tracing::warn!(workspace = %self.workspace, %error, "recording terminal activity failed");
        }
    }
}

impl TerminalBridge {
    /// The largest frame accepted from a client: a generous paste.
    const MAX_FRAME: usize = 1 << 20;
    /// WebSocket close code: normal closure.
    const NORMAL: u16 = 1000;
    /// WebSocket close code: a frame did not make sense.
    const INVALID: u16 = 1007;

    async fn run(mut self) {
        self.serve().await;
        if let Some(presence) = &self.access.presence {
            presence.touch().await;
        }
    }

    async fn serve(&mut self) {
        let closing = loop {
            tokio::select! {
                () = async {
                    match self.access.presence.as_mut() {
                        Some(presence) => presence.due().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Some(presence) = &self.access.presence {
                        presence.touch().await;
                    }
                }
                () = async {
                    match self.access.guard.as_mut() {
                        Some(guard) => guard.due().await,
                        None => std::future::pending().await,
                    }
                } => {
                    let holds = match &self.access.guard {
                        Some(guard) => guard.holds().await,
                        None => true,
                    };
                    if !holds {
                        self.access.writable = false;
                        self.access.guard = None;
                    }
                }
                event = self.session.next() => match event {
                    Some(TerminalEvent::Output(data)) => {
                        if self.socket.send(Message::Binary(data.into())).await.is_err() {
                            return;
                        }
                    }
                    Some(TerminalEvent::Exited(outcome)) => {
                        break self.finish(TerminalServerFrame::from(outcome)).await;
                    }
                    None => break self.finish(TerminalServerFrame::lost()).await,
                },
                frame = self.socket.recv() => match frame {
                    Some(Ok(Message::Binary(data))) => {
                        // A read-only terminal, or one whose holder handed the task back, drops
                        // what is typed.
                        if self.access.writable
                            && self.session.write(data.to_vec()).await.is_err()
                        {
                            break self.finish(TerminalServerFrame::lost()).await;
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if !Self::control(&self.session, text.as_str()).await {
                            break Self::INVALID;
                        }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                },
            }
        };
        let frame = CloseFrame {
            code: closing,
            reason: "".into(),
        };
        let _ = self.socket.send(Message::Close(Some(frame))).await;
    }

    /// Sends the closing text frame; the close code to follow it with.
    async fn finish(&mut self, exit: TerminalServerFrame) -> u16 {
        if let Ok(json) = serde_json::to_string(&exit) {
            let _ = self.socket.send(Message::Text(json.into())).await;
        }
        Self::NORMAL
    }

    /// Applies a client control frame; false if it was not one.
    async fn control(session: &TerminalSession, text: &str) -> bool {
        match serde_json::from_str::<TerminalClientFrame>(text) {
            Ok(TerminalClientFrame::Resize { cols, rows }) => {
                match TerminalSize::new(u32::from(cols), u32::from(rows)) {
                    Ok(size) => session.resize(size).await.is_ok(),
                    Err(_) => false,
                }
            }
            _ => false,
        }
    }
}
