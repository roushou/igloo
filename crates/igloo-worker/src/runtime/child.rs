use std::os::unix::process::ExitStatusExt;
use std::process::Stdio;
use std::time::Duration;

use igloo_core::process::OutputStream;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{ExitOutcome, OutputChunk, RuntimeError};

/// A spawned process whose output is pumped into chunks until it exits, times out or is
/// cancelled.
pub(super) struct Supervised {
    child: Child,
    /// Run to stop the process; without one, the child is killed directly.
    stop: Option<Command>,
}

/// Copies one output stream into chunks, tracking byte offsets.
struct Pump<'a> {
    stream: OutputStream,
    output: &'a mpsc::Sender<OutputChunk>,
}

impl Supervised {
    /// Spawns `command` with no stdin and piped output. `stop`, when given, is run to stop it
    /// on timeout or cancellation, for processes that only proxy the real one.
    pub(super) fn spawn(
        command: &mut Command,
        stop: Option<Command>,
    ) -> Result<Self, RuntimeError> {
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(RuntimeError::Spawn)?;
        Ok(Self { child, stop })
    }

    /// Streams output to `output` and waits for the end. A process killed by a signal exits
    /// with 128 plus the signal number.
    pub(super) async fn wait(
        mut self,
        timeout: Duration,
        output: &mpsc::Sender<OutputChunk>,
        cancel: &CancellationToken,
    ) -> Result<ExitOutcome, RuntimeError> {
        let stdout = self.child.stdout.take();
        let stderr = self.child.stderr.take();
        let pumps = async {
            let stdout = async {
                if let Some(reader) = stdout {
                    Pump::new(OutputStream::Stdout, output).drain(reader).await;
                }
            };
            let stderr = async {
                if let Some(reader) = stderr {
                    Pump::new(OutputStream::Stderr, output).drain(reader).await;
                }
            };
            tokio::join!(stdout, stderr);
        };
        let waiting = async {
            let ended = tokio::select! {
                status = self.child.wait() => return status.map(|status| {
                    ExitOutcome::Exited(
                        status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0)),
                    )
                }),
                () = tokio::time::sleep(timeout) => ExitOutcome::TimedOut,
                () = cancel.cancelled() => ExitOutcome::Cancelled,
            };
            self.halt().await?;
            Ok(ended)
        };
        let (outcome, ()) = tokio::join!(waiting, pumps);
        Ok(outcome?)
    }

    async fn halt(&mut self) -> std::io::Result<()> {
        if let Some(stop) = &mut self.stop {
            let stopped = stop
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
            if stopped.is_ok_and(|status| status.success()) {
                self.child.wait().await?;
                return Ok(());
            }
        }
        self.child.kill().await
    }
}

impl<'a> Pump<'a> {
    const CHUNK: usize = 8 * 1024;

    fn new(stream: OutputStream, output: &'a mpsc::Sender<OutputChunk>) -> Self {
        Self { stream, output }
    }

    /// Reads until end of stream; stops early if nobody listens anymore.
    async fn drain(self, mut reader: impl AsyncRead + Unpin) {
        let mut offset = 0u64;
        let mut buffer = vec![0u8; Self::CHUNK];
        loop {
            let read = match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(read) => read,
            };
            let data = buffer.get(..read).unwrap_or_default().to_vec();
            let chunk = OutputChunk {
                stream: self.stream,
                offset,
                data,
            };
            offset += read as u64;
            if self.output.send(chunk).await.is_err() {
                return;
            }
        }
    }
}
