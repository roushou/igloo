use std::io;
use std::os::fd::OwnedFd;
use std::process::Stdio;
use std::time::Duration;

use igloo_core::terminal::TerminalSize;
use rustix::fs::{Mode, OFlags};
use rustix::process::{Pid, Signal};
use rustix::pty::OpenptFlags;
use rustix::termios::Winsize;
use tokio::io::unix::AsyncFd;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{ExitOutcome, RuntimeError, TerminalInput, TerminalIo};

/// The controlling side of a pseudo-terminal: bytes written here are typed into the process,
/// bytes read are what it printed.
struct Master {
    fd: AsyncFd<OwnedFd>,
}

/// A process whose standard streams and controlling terminal are the slave side of a
/// pseudo-terminal, driven through the master side until it exits or is cancelled.
pub(super) struct PtyProcess {
    leader: Leader,
    master: Master,
}

/// The process leading a terminal's session. Invariant: it is killed when dropped.
struct Leader {
    child: Child,
    /// Run to stop the process; without one, its process group is killed directly.
    stop: Option<Command>,
}

impl Master {
    const CHUNK: usize = 16 * 1024;

    /// A new pseudo-terminal of `size`: the master side, and the slave side to give to a process.
    fn open(size: TerminalSize) -> io::Result<(Self, OwnedFd)> {
        let master = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
        rustix::pty::grantpt(&master)?;
        rustix::pty::unlockpt(&master)?;
        let name = rustix::pty::ptsname(&master, Vec::new())?;
        let slave = rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let flags = rustix::fs::fcntl_getfl(&master)?;
        rustix::fs::fcntl_setfl(&master, flags | OFlags::NONBLOCK)?;
        let master = Self {
            fd: AsyncFd::new(master)?,
        };
        master.resize(size)?;
        Ok((master, slave))
    }

    fn resize(&self, size: TerminalSize) -> io::Result<()> {
        let winsize = Winsize {
            ws_row: size.rows(),
            ws_col: size.cols(),
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        Ok(rustix::termios::tcsetwinsize(self.fd.get_ref(), winsize)?)
    }

    /// Reads what the process printed; `0` once every slave descriptor is closed.
    async fn read(&self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut ready = self.fd.readable().await?;
            match ready
                .try_io(|fd| rustix::io::read(fd.get_ref(), &mut *buffer).map_err(Into::into))
            {
                Ok(Err(error))
                    if error.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) =>
                {
                    return Ok(0);
                }
                Ok(read) => return read,
                Err(_would_block) => {}
            }
        }
    }

    async fn write_all(&self, mut data: &[u8]) -> io::Result<()> {
        while !data.is_empty() {
            let mut ready = self.fd.writable().await?;
            match ready.try_io(|fd| rustix::io::write(fd.get_ref(), data).map_err(Into::into)) {
                Ok(written) => data = data.get(written?..).unwrap_or_default(),
                Err(_would_block) => {}
            }
        }
        Ok(())
    }

    /// Forwards output to `output` until the terminal closes or nobody listens.
    async fn pump_output(&self, output: &mpsc::Sender<Vec<u8>>) {
        let mut buffer = vec![0u8; Self::CHUNK];
        loop {
            match self.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    let data = buffer.get(..read).unwrap_or_default().to_vec();
                    if output.send(data).await.is_err() {
                        return;
                    }
                }
            }
        }
    }

    /// Applies input until its sender is dropped. A process that stopped reading drops what is
    /// typed at it.
    async fn pump_input(&self, input: &mut mpsc::Receiver<TerminalInput>) {
        while let Some(message) = input.recv().await {
            let applied = match message {
                TerminalInput::Data(data) => self.write_all(&data).await,
                TerminalInput::Resize(size) => self.resize(size),
            };
            if let Err(error) = applied {
                tracing::debug!(%error, "terminal input dropped");
            }
        }
    }
}

impl PtyProcess {
    /// How long output the process left behind is collected after it exits.
    const DRAIN: Duration = Duration::from_millis(500);

    /// Spawns `command` on a new pseudo-terminal of `size`. `stop`, when given, is run to stop
    /// it on cancellation, for processes that only proxy the real one.
    pub(super) fn spawn(
        command: &mut Command,
        size: TerminalSize,
        stop: Option<Command>,
    ) -> Result<Self, RuntimeError> {
        let (master, slave) = Master::open(size)?;
        command
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave))
            .kill_on_drop(true);
        // SAFETY: the closure only makes the async-signal-safe system calls `setsid` and
        // `ioctl`, allocates nothing and touches no shared state.
        #[allow(
            unsafe_code,
            reason = "a controlling terminal can only be set up between fork and exec"
        )]
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(std::io::stdin())?;
                Ok(())
            });
        }
        let child = command.spawn().map_err(RuntimeError::Spawn)?;
        // The command still holds the slave descriptors; release them so the master sees the
        // terminal close when the process exits.
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        Ok(Self {
            leader: Leader { child, stop },
            master,
        })
    }

    /// Connects the process to `io` and waits for the end. Output the process printed before
    /// exiting is delivered before this returns. The process is killed when `cancel` fires or
    /// when `io.input` is closed.
    pub(super) async fn run(
        self,
        io: TerminalIo,
        cancel: &CancellationToken,
    ) -> Result<ExitOutcome, RuntimeError> {
        let Self { mut leader, master } = self;
        let TerminalIo { mut input, output } = io;
        let reader = master.pump_output(&output);
        let writer = master.pump_input(&mut input);
        tokio::pin!(reader, writer);
        let mut reading = true;
        let ended = loop {
            tokio::select! {
                status = leader.child.wait() => break status.map(ExitOutcome::from),
                () = cancel.cancelled() => break Ok(ExitOutcome::Cancelled),
                () = &mut writer => break Ok(ExitOutcome::Cancelled),
                () = &mut reader, if reading => reading = false,
            }
        };
        let outcome = ended?;
        if outcome == ExitOutcome::Cancelled {
            leader.halt().await?;
        }
        if reading {
            // Descendants may keep the terminal open; they get a moment, not forever.
            let _ = tokio::time::timeout(Self::DRAIN, &mut reader).await;
        }
        Ok(outcome)
    }
}

impl Leader {
    async fn halt(&mut self) -> io::Result<()> {
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
        let group = self
            .child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .and_then(Pid::from_raw);
        if let Some(group) = group {
            // The child leads its own session and process group; a group already gone is fine.
            let _ = rustix::process::kill_process_group(group, Signal::KILL);
        }
        self.child.kill().await
    }
}
