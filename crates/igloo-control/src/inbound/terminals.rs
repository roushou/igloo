//! Interactive terminals: the meeting point of callers (the REST WebSocket) and worker
//! sessions (the gateway). A caller opens a terminal on the worker running a sandbox and
//! exchanges bytes with it; the worker's session delivers the process's output and exit.
//!
//! Terminals are live connections, not state: they never enter the event log and end with the
//! worker connection that carries them.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use igloo_core::ErrorCode;
use igloo_core::sandbox::SandboxId;
use igloo_core::terminal::{TerminalId, TerminalOutcome, TerminalSize};
use igloo_core::worker::WorkerId;
use igloo_worker_protocol::v1;
use igloo_worker_protocol::v1::connect_response::Message as Downlink;
use tokio::sync::mpsc;
use tonic::Status;

/// The channel to a worker's session.
pub(crate) type WorkerChannel = mpsc::Sender<Result<v1::ConnectResponse, Status>>;

/// Routes terminal traffic between callers and worker sessions. Clones share the same terminals.
///
/// Invariants: a terminal is delivered to at most one caller and only ever receives messages from
/// the worker connection it was opened on; it appears in the hub from before its open message
/// is sent until its exit is delivered, its caller drops it, or its connection ends.
#[derive(Clone, Default)]
pub struct TerminalHub {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    next_serial: u64,
    links: HashMap<WorkerId, (u64, WorkerChannel)>,
    live: HashMap<TerminalId, Live>,
}

struct Live {
    worker: WorkerId,
    serial: u64,
    events: mpsc::Sender<TerminalEvent>,
}

/// What to run in a terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalLaunch {
    /// The sandbox to run in.
    pub sandbox: SandboxId,
    /// The program and its arguments; empty runs the worker's default shell.
    pub argv: Vec<String>,
    /// Environment layered over the sandbox's.
    pub env: BTreeMap<String, String>,
    /// The screen to start with.
    pub size: TerminalSize,
}

/// What a terminal produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    /// Bytes the process printed, in order.
    Output(Vec<u8>),
    /// The process ended; the last event of a terminal.
    Exited(TerminalOutcome),
}

/// Why a terminal could not be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TerminalError {
    /// No worker connection can reach the sandbox.
    #[error("the sandbox's worker is not connected")]
    WorkerUnavailable,
    /// The terminal or its worker connection ended.
    #[error("the terminal has ended")]
    Ended,
}

impl ErrorCode for TerminalError {
    fn code(&self) -> &'static str {
        match self {
            Self::WorkerUnavailable => "terminal.worker_unavailable",
            Self::Ended => "terminal.ended",
        }
    }
}

/// One caller's end of an open terminal.
///
/// Invariant: dropping it closes the terminal on the worker, best effort: if the worker's
/// channel is full the close is lost and the process lives until its connection ends.
pub struct TerminalSession {
    id: TerminalId,
    hub: TerminalHub,
    worker: WorkerChannel,
    events: mpsc::Receiver<TerminalEvent>,
}

/// A worker session's registration with the hub. Dropping it ends the terminals opened on its
/// connection.
pub(crate) struct WorkerLink {
    hub: TerminalHub,
    worker: WorkerId,
    serial: u64,
}

impl TerminalHub {
    /// Events waiting for a caller that reads slowly; a terminal whose caller falls this far
    /// behind is closed.
    const BACKLOG: usize = 64;

    /// A hub with no workers.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the session of `worker`, reached through `channel`, replacing an earlier one.
    pub(crate) fn link(&self, worker: WorkerId, channel: WorkerChannel) -> WorkerLink {
        let mut state = self.state();
        state.next_serial += 1;
        let serial = state.next_serial;
        state.links.insert(worker, (serial, channel));
        WorkerLink {
            hub: self.clone(),
            worker,
            serial,
        }
    }

    /// Opens terminal `id` on `worker`.
    pub async fn open(
        &self,
        id: TerminalId,
        worker: WorkerId,
        launch: TerminalLaunch,
    ) -> Result<TerminalSession, TerminalError> {
        let (events_tx, events) = mpsc::channel(Self::BACKLOG);
        let channel = {
            let mut state = self.state();
            let (serial, channel) = state
                .links
                .get(&worker)
                .cloned()
                .ok_or(TerminalError::WorkerUnavailable)?;
            state.live.insert(
                id,
                Live {
                    worker,
                    serial,
                    events: events_tx,
                },
            );
            channel
        };
        let session = TerminalSession {
            id,
            hub: self.clone(),
            worker: channel,
            events,
        };
        let open = v1::OpenTerminal {
            terminal_id: id.to_string(),
            sandbox_id: launch.sandbox.to_string(),
            argv: launch.argv,
            env: launch.env.into_iter().collect(),
            size: Some(launch.size.into()),
        };
        session.send(Downlink::OpenTerminal(open)).await?;
        Ok(session)
    }

    /// Forgets terminal `id`; whether it was still open.
    fn forget(&self, id: TerminalId) -> bool {
        self.state().live.remove(&id).is_some()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl TerminalSession {
    /// The terminal's id.
    #[must_use]
    pub const fn id(&self) -> TerminalId {
        self.id
    }

    /// The next thing the terminal produced; `None` once it ended without an exit (the worker's
    /// connection dropped, or this caller fell too far behind).
    pub async fn next(&mut self) -> Option<TerminalEvent> {
        self.events.recv().await
    }

    /// Types `data` into the process.
    pub async fn write(&self, data: Vec<u8>) -> Result<(), TerminalError> {
        let input = v1::TerminalInput {
            terminal_id: self.id.to_string(),
            data,
        };
        self.send(Downlink::TerminalInput(input)).await
    }

    /// Tells the process its screen is now `size`.
    pub async fn resize(&self, size: TerminalSize) -> Result<(), TerminalError> {
        let resize = v1::ResizeTerminal {
            terminal_id: self.id.to_string(),
            size: Some(size.into()),
        };
        self.send(Downlink::ResizeTerminal(resize)).await
    }

    async fn send(&self, message: Downlink) -> Result<(), TerminalError> {
        let response = v1::ConnectResponse {
            message: Some(message),
        };
        self.worker
            .send(Ok(response))
            .await
            .map_err(|_| TerminalError::Ended)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if self.hub.forget(self.id) {
            let close = v1::CloseTerminal {
                terminal_id: self.id.to_string(),
            };
            let response = v1::ConnectResponse {
                message: Some(Downlink::CloseTerminal(close)),
            };
            let _ = self.worker.try_send(Ok(response));
        }
    }
}

impl WorkerLink {
    /// Delivers output a worker printed for one of its terminals; anything else is ignored. A
    /// caller that cannot keep up loses the terminal.
    pub(crate) fn output(&self, output: v1::TerminalOutput) {
        let Ok(id) = output.terminal_id.parse::<TerminalId>() else {
            return;
        };
        let mut state = self.hub.state();
        let Some(live) = state.live.get(&id).filter(|live| self.owns(live)) else {
            return;
        };
        // The last slot is kept for the exit, so a full queue still ends with one.
        let delivered = live.events.capacity() > 1
            && live
                .events
                .try_send(TerminalEvent::Output(output.data))
                .is_ok();
        if !delivered {
            state.live.remove(&id);
            self.close(id, &state);
        }
    }

    /// Delivers the end of one of the worker's terminals; anything else is ignored.
    pub(crate) fn exit(&self, exit: &v1::TerminalExit) {
        let Ok(id) = exit.terminal_id.parse::<TerminalId>() else {
            return;
        };
        let outcome = TerminalOutcome::try_from(exit);
        let mut state = self.hub.state();
        if !state.live.get(&id).is_some_and(|live| self.owns(live)) {
            return;
        }
        if let Some(live) = state.live.remove(&id)
            && let Ok(outcome) = outcome
        {
            let _ = live.events.try_send(TerminalEvent::Exited(outcome));
        }
    }

    fn owns(&self, live: &Live) -> bool {
        live.worker == self.worker && live.serial == self.serial
    }

    /// Asks the worker to end terminal `id`, best effort.
    fn close(&self, id: TerminalId, state: &State) {
        if let Some((serial, channel)) = state.links.get(&self.worker)
            && *serial == self.serial
        {
            let close = v1::CloseTerminal {
                terminal_id: id.to_string(),
            };
            let response = v1::ConnectResponse {
                message: Some(Downlink::CloseTerminal(close)),
            };
            let _ = channel.try_send(Ok(response));
        }
    }
}

impl Drop for WorkerLink {
    fn drop(&mut self) {
        let mut state = self.hub.state();
        if state
            .links
            .get(&self.worker)
            .is_some_and(|(serial, _)| *serial == self.serial)
        {
            state.links.remove(&self.worker);
        }
        let (worker, serial) = (self.worker, self.serial);
        state
            .live
            .retain(|_, live| live.worker != worker || live.serial != serial);
    }
}

#[cfg(test)]
mod tests {
    use igloo_core::terminal::TerminalFailure;
    use uuid::Uuid;

    use super::*;

    fn worker(n: u128) -> WorkerId {
        igloo_core::Id::from_uuid(Uuid::from_u128(n))
    }

    fn terminal(n: u128) -> TerminalId {
        igloo_core::Id::from_uuid(Uuid::from_u128(n))
    }

    fn launch() -> TerminalLaunch {
        TerminalLaunch {
            sandbox: igloo_core::Id::from_uuid(Uuid::from_u128(9)),
            argv: vec!["bash".into()],
            env: BTreeMap::new(),
            size: TerminalSize::new(100, 30).expect("size"),
        }
    }

    fn output(id: TerminalId, text: &str) -> v1::TerminalOutput {
        v1::TerminalOutput {
            terminal_id: id.to_string(),
            data: text.as_bytes().to_vec(),
        }
    }

    async fn sent(downlink: &mut mpsc::Receiver<Result<v1::ConnectResponse, Status>>) -> Downlink {
        downlink
            .recv()
            .await
            .expect("a message")
            .expect("not an error")
            .message
            .expect("a body")
    }

    /// A hub with `worker` linked; the receiver is what the worker is sent.
    fn linked(
        worker: WorkerId,
    ) -> (
        TerminalHub,
        WorkerLink,
        mpsc::Receiver<Result<v1::ConnectResponse, Status>>,
    ) {
        let hub = TerminalHub::new();
        let (channel, downlink) = mpsc::channel(8);
        let link = hub.link(worker, channel);
        (hub, link, downlink)
    }

    #[tokio::test]
    async fn opening_asks_the_worker_and_input_follows_in_order() {
        let (hub, _link, mut downlink) = linked(worker(1));
        let session = hub
            .open(terminal(1), worker(1), launch())
            .await
            .expect("open");
        session.write(b"ls\n".to_vec()).await.expect("write");
        session
            .resize(TerminalSize::new(132, 43).expect("size"))
            .await
            .expect("resize");

        let Downlink::OpenTerminal(open) = sent(&mut downlink).await else {
            panic!("the open comes first");
        };
        assert_eq!(open.terminal_id, terminal(1).to_string());
        assert_eq!(open.argv, ["bash"]);
        assert_eq!(
            open.size,
            Some(v1::TerminalSize {
                cols: 100,
                rows: 30
            })
        );
        let Downlink::TerminalInput(input) = sent(&mut downlink).await else {
            panic!("then input");
        };
        assert_eq!(input.data, b"ls\n");
        let Downlink::ResizeTerminal(resize) = sent(&mut downlink).await else {
            panic!("then the resize");
        };
        assert_eq!(
            resize.size,
            Some(v1::TerminalSize {
                cols: 132,
                rows: 43
            })
        );
    }

    #[tokio::test]
    async fn output_and_exit_reach_the_caller_in_order() {
        let (hub, link, _downlink) = linked(worker(1));
        let mut session = hub
            .open(terminal(1), worker(1), launch())
            .await
            .expect("open");
        link.output(output(terminal(1), "hel"));
        link.output(output(terminal(1), "lo"));
        link.exit(&v1::TerminalExit::new(
            terminal(1),
            TerminalOutcome::Exited(4),
        ));

        assert_eq!(
            session.next().await,
            Some(TerminalEvent::Output(b"hel".to_vec()))
        );
        assert_eq!(
            session.next().await,
            Some(TerminalEvent::Output(b"lo".to_vec()))
        );
        assert_eq!(
            session.next().await,
            Some(TerminalEvent::Exited(TerminalOutcome::Exited(4)))
        );
        assert_eq!(session.next().await, None, "nothing follows the exit");
    }

    #[tokio::test]
    async fn a_worker_cannot_speak_for_another_workers_terminal() {
        let (hub, link, _downlink) = linked(worker(1));
        let (channel, _other_downlink) = mpsc::channel(8);
        let other = hub.link(worker(2), channel);
        let mut session = hub
            .open(terminal(1), worker(1), launch())
            .await
            .expect("open");

        other.output(output(terminal(1), "forged"));
        other.exit(&v1::TerminalExit::new(
            terminal(1),
            TerminalOutcome::Exited(0),
        ));
        link.output(output(terminal(1), "real"));

        assert_eq!(
            session.next().await,
            Some(TerminalEvent::Output(b"real".to_vec())),
            "the forgeries reached nobody"
        );
    }

    #[tokio::test]
    async fn dropping_the_session_closes_the_terminal() {
        let (hub, link, mut downlink) = linked(worker(1));
        let session = hub
            .open(terminal(1), worker(1), launch())
            .await
            .expect("open");
        assert!(matches!(
            sent(&mut downlink).await,
            Downlink::OpenTerminal(_)
        ));
        drop(session);
        let Downlink::CloseTerminal(close) = sent(&mut downlink).await else {
            panic!("the worker is told to close it");
        };
        assert_eq!(close.terminal_id, terminal(1).to_string());
        link.output(output(terminal(1), "late"));
    }

    #[tokio::test]
    async fn losing_the_worker_connection_ends_its_terminals() {
        let (hub, link, _downlink) = linked(worker(1));
        let mut session = hub
            .open(terminal(1), worker(1), launch())
            .await
            .expect("open");
        drop(link);
        assert_eq!(session.next().await, None);
        assert_eq!(
            hub.open(terminal(2), worker(1), launch()).await.err(),
            Some(TerminalError::WorkerUnavailable)
        );
    }

    #[tokio::test]
    async fn a_reconnection_does_not_inherit_or_lose_terminals() {
        let (hub, old, _old_downlink) = linked(worker(1));
        let mut session = hub
            .open(terminal(1), worker(1), launch())
            .await
            .expect("open");
        let (channel, mut downlink) = mpsc::channel(8);
        let _new = hub.link(worker(1), channel);
        drop(old);
        assert_eq!(
            session.next().await,
            None,
            "the old connection's terminal ended"
        );
        assert!(
            hub.open(terminal(2), worker(1), launch()).await.is_ok(),
            "the new connection still serves terminals"
        );
        assert!(matches!(
            sent(&mut downlink).await,
            Downlink::OpenTerminal(_)
        ));
    }

    #[tokio::test]
    async fn a_caller_that_falls_behind_loses_the_terminal_but_not_its_exit_slot() {
        let (hub, link, mut downlink) = linked(worker(1));
        let mut session = hub
            .open(terminal(1), worker(1), launch())
            .await
            .expect("open");
        assert!(matches!(
            sent(&mut downlink).await,
            Downlink::OpenTerminal(_)
        ));
        for _ in 0..TerminalHub::BACKLOG * 2 {
            link.output(output(terminal(1), "x"));
        }
        let Downlink::CloseTerminal(_) = sent(&mut downlink).await else {
            panic!("the worker is told to close the terminal");
        };
        let mut received = 0;
        while session.next().await.is_some() {
            received += 1;
        }
        assert_eq!(received, TerminalHub::BACKLOG - 1);
    }

    #[tokio::test]
    async fn a_terminal_that_fails_to_start_reports_why() {
        let (hub, link, _downlink) = linked(worker(1));
        let mut session = hub
            .open(terminal(1), worker(1), launch())
            .await
            .expect("open");
        let failed = TerminalOutcome::Failed(TerminalFailure::SandboxUnavailable);
        link.exit(&v1::TerminalExit::new(terminal(1), failed));
        assert_eq!(session.next().await, Some(TerminalEvent::Exited(failed)));
    }
}
