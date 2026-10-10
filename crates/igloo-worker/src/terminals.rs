use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use igloo_core::sandbox::SandboxId;
use igloo_core::terminal::{TerminalFailure, TerminalId, TerminalOutcome, TerminalSize};
use igloo_worker_protocol::v1;
use igloo_worker_protocol::v1::connect_request::Message as Uplink;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::reconciler::Sandboxes;
use crate::runtime::{
    ExitOutcome, LocalSandbox, SandboxRuntime, TerminalInput, TerminalIo, TerminalProcess,
};

/// The interactive terminals running on this worker.
///
/// Invariants: a terminal in the table has a running task; every task ends with exactly one
/// [`v1::TerminalExit`] returned by [`Terminals::next_ended`], after all of its output was
/// queued on the uplink.
pub(crate) struct Terminals {
    open: HashMap<TerminalId, Open>,
    tasks: JoinSet<v1::TerminalExit>,
    runtime: Arc<dyn SandboxRuntime>,
    uplink: mpsc::Sender<Uplink>,
}

/// The handles to one running terminal.
struct Open {
    input: mpsc::Sender<TerminalInput>,
    cancel: CancellationToken,
}

/// One terminal's task: runs the process and forwards its output to the uplink.
struct Run {
    id: TerminalId,
    sandbox: LocalSandbox,
    process: TerminalProcess,
    input: mpsc::Receiver<TerminalInput>,
    cancel: CancellationToken,
    runtime: Arc<dyn SandboxRuntime>,
    uplink: mpsc::Sender<Uplink>,
}

impl Terminals {
    /// Input messages waiting for a process that has stopped reading; more are dropped.
    const INPUT_BACKLOG: usize = 256;
    /// Output chunks waiting for the uplink.
    const OUTPUT_BACKLOG: usize = 64;
    /// The program run when a request names none.
    const DEFAULT_PROGRAM: &'static str = "sh";
    /// The terminal type announced to the process unless the request sets one.
    const DEFAULT_TERM: &'static str = "xterm-256color";

    pub(crate) fn new(runtime: Arc<dyn SandboxRuntime>, uplink: mpsc::Sender<Uplink>) -> Self {
        Self {
            open: HashMap::new(),
            tasks: JoinSet::new(),
            runtime,
            uplink,
        }
    }

    /// Whether any terminal task is running or waiting to be collected.
    pub(crate) fn has_tasks(&self) -> bool {
        !self.tasks.is_empty()
    }

    /// Opens the terminal `request` asks for in `sandboxes`. Returns the exit to report now
    /// when it cannot start; an id that is already open is ignored.
    pub(crate) fn open(
        &mut self,
        request: v1::OpenTerminal,
        sandboxes: &Sandboxes,
    ) -> Option<v1::TerminalExit> {
        let Ok(id) = request.terminal_id.parse::<TerminalId>() else {
            tracing::warn!(terminal = request.terminal_id, "terminal id is not valid");
            return None;
        };
        if self.open.contains_key(&id) {
            return None;
        }
        let failed = |failure| Some(v1::TerminalExit::new(id, TerminalOutcome::Failed(failure)));
        let sandbox = request
            .sandbox_id
            .parse::<SandboxId>()
            .ok()
            .and_then(|sandbox| sandboxes.get(sandbox).cloned());
        let Some(sandbox) = sandbox else {
            return failed(TerminalFailure::SandboxUnavailable);
        };
        let size = request
            .size
            .as_ref()
            .map_or(Ok(TerminalSize::DEFAULT), TerminalSize::try_from);
        let Ok(size) = size else {
            return failed(TerminalFailure::ExecutionError);
        };
        let mut argv = request.argv;
        if argv.is_empty() {
            argv.push(Self::DEFAULT_PROGRAM.to_owned());
        }
        let mut env: BTreeMap<String, String> = request.env.into_iter().collect();
        env.entry("TERM".to_owned())
            .or_insert_with(|| Self::DEFAULT_TERM.to_owned());

        let (input_tx, input) = mpsc::channel(Self::INPUT_BACKLOG);
        let cancel = CancellationToken::new();
        let run = Run {
            id,
            sandbox,
            process: TerminalProcess { argv, env, size },
            input,
            cancel: cancel.clone(),
            runtime: Arc::clone(&self.runtime),
            uplink: self.uplink.clone(),
        };
        self.open.insert(
            id,
            Open {
                input: input_tx,
                cancel,
            },
        );
        self.tasks.spawn(run.run());
        None
    }

    /// Types `input.data` into its terminal. Dropped when the terminal is unknown, or when its
    /// process lets [`Terminals::INPUT_BACKLOG`] messages pile up unread.
    pub(crate) fn input(&self, input: v1::TerminalInput) {
        if let Some(open) = self.find(&input.terminal_id)
            && open
                .input
                .try_send(TerminalInput::Data(input.data))
                .is_err()
        {
            tracing::warn!(terminal = input.terminal_id, "terminal input dropped");
        }
    }

    /// Resizes the screen of a terminal; an unknown terminal or invalid size is ignored.
    pub(crate) fn resize(&self, resize: &v1::ResizeTerminal) {
        let size = resize.size.as_ref().map(TerminalSize::try_from);
        if let (Some(open), Some(Ok(size))) = (self.find(&resize.terminal_id), size) {
            let _ = open.input.try_send(TerminalInput::Resize(size));
        }
    }

    /// Ends a terminal at the server's request; an unknown terminal is ignored.
    pub(crate) fn close(&mut self, terminal: &str) {
        if let Some(open) = terminal
            .parse::<TerminalId>()
            .ok()
            .and_then(|id| self.open.get(&id))
        {
            open.cancel.cancel();
        }
    }

    /// Ends every terminal, for a connection that dropped: nobody is left to read them.
    pub(crate) fn close_all(&mut self) {
        for open in self.open.values() {
            open.cancel.cancel();
        }
    }

    /// Waits for the next terminal to end and returns its exit. The terminal's output was
    /// queued on the uplink before this returns.
    pub(crate) async fn next_ended(&mut self) -> Option<v1::TerminalExit> {
        let exit = self.tasks.join_next().await?.ok()?;
        if let Ok(id) = exit.terminal_id.parse::<TerminalId>() {
            self.open.remove(&id);
        }
        Some(exit)
    }

    fn find(&self, terminal: &str) -> Option<&Open> {
        terminal
            .parse::<TerminalId>()
            .ok()
            .and_then(|id| self.open.get(&id))
    }
}

impl Run {
    async fn run(self) -> v1::TerminalExit {
        let Self {
            id,
            sandbox,
            process,
            input,
            cancel,
            runtime,
            uplink,
        } = self;
        let (output, mut chunks) = mpsc::channel::<Vec<u8>>(Terminals::OUTPUT_BACKLOG);
        let forward = async {
            while let Some(data) = chunks.recv().await {
                let message = v1::TerminalOutput {
                    terminal_id: id.to_string(),
                    data,
                };
                if uplink.send(Uplink::TerminalOutput(message)).await.is_err() {
                    return;
                }
            }
        };
        let io = TerminalIo { input, output };
        let (outcome, ()) = tokio::join!(
            runtime.exec_terminal(&sandbox, process, io, cancel),
            forward
        );
        let outcome = match outcome {
            Ok(ExitOutcome::Exited(code)) => TerminalOutcome::Exited(code),
            Ok(ExitOutcome::Cancelled | ExitOutcome::TimedOut) => {
                TerminalOutcome::Failed(TerminalFailure::Closed)
            }
            Err(error) => {
                tracing::warn!(terminal = %id, %error, "the terminal could not run");
                TerminalOutcome::Failed(TerminalFailure::ExecutionError)
            }
        };
        v1::TerminalExit::new(id, outcome)
    }
}
