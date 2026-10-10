use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::Response;
use igloo_api::problem::Problem;
use igloo_api::terminal::{TerminalClientFrame, TerminalRequest, TerminalServerFrame};
use igloo_core::Resource as _;
use igloo_core::sandbox::SandboxPhase;
use igloo_core::terminal::TerminalSize;
use igloo_core::workspace::WorkspaceId;
use tokio::time::{Interval, MissedTickBehavior};

use super::auth::TerminalCaller;
use super::sandboxes::{load, parse_id};
use super::{ApiError, ApiState};
use crate::app::{AppError, CommandBus, RequestContext};
use crate::inbound::{TerminalEvent, TerminalLaunch, TerminalSession};
use crate::ports::IdGeneratorExt as _;
use crate::workspaces::TouchWorkspace;

/// Opens an interactive terminal in a running sandbox: upgrades to a WebSocket speaking the
/// subprotocol `igloo.terminal.v1`.
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
    let launch = TerminalLaunch {
        sandbox: igloo_core::Entity::id(&sandbox),
        argv: request.command,
        env: state
            .workspace_secrets
            .terminal_env(igloo_core::Entity::id(&sandbox))
            .await?,
        size: request.size,
    };
    let session = state
        .terminals
        .open(state.ids.next(), worker, launch)
        .await?;
    let workspace = state
        .workspace_secrets
        .workspace_of(igloo_core::Entity::id(&sandbox))
        .await?;
    let presence = workspace.map(|workspace| Presence::new(state.bus.clone(), workspace, context));
    Ok(upgrade
        .protocols([TerminalRequest::PROTOCOL])
        .max_message_size(TerminalBridge::MAX_FRAME)
        .on_upgrade(move |socket| {
            TerminalBridge {
                socket,
                session,
                presence,
            }
            .run()
        }))
}

/// Carries one terminal over one WebSocket until either ends.
struct TerminalBridge {
    socket: WebSocket,
    session: TerminalSession,
    presence: Option<Presence>,
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
        if let Some(presence) = &self.presence {
            presence.touch().await;
        }
    }

    async fn serve(&mut self) {
        let closing = loop {
            tokio::select! {
                () = async {
                    match self.presence.as_mut() {
                        Some(presence) => presence.due().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Some(presence) = &self.presence {
                        presence.touch().await;
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
                        if self.session.write(data.to_vec()).await.is_err() {
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
