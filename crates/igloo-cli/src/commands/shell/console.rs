//! The local terminal `igloo shell` is attached to.

use std::io::{self, Read as _, Write as _};

use rustix::termios::{self, OptionalActions, Termios};
use tokio::sync::mpsc;

/// The terminal `igloo shell` runs in. While it is attached to a terminal it is in raw mode:
/// every key, Ctrl-C included, goes to the remote shell.
///
/// Invariant: the terminal's mode is the one it had before [`Console::attach`] once the
/// console is dropped.
pub(super) struct Console {
    /// The mode to restore; set only when standard input is a terminal.
    original: Option<Termios>,
    keys: mpsc::Receiver<Vec<u8>>,
}

impl Console {
    /// The screen assumed when standard output is not a terminal.
    const FALLBACK: (u16, u16) = (80, 24);
    /// Bytes read from the keyboard at once.
    const CHUNK: usize = 4096;
    /// Chunks of keyboard input waiting to be sent.
    const BACKLOG: usize = 16;

    /// Attaches to the process's terminal: raw mode when standard input is one.
    pub(super) fn attach() -> io::Result<Self> {
        let stdin = io::stdin();
        if !termios::isatty(&stdin) {
            return Ok(Self {
                original: None,
                keys: Self::read_keys(),
            });
        }
        let original = termios::tcgetattr(&stdin)?;
        let mut raw = original.clone();
        raw.make_raw();
        termios::tcsetattr(&stdin, OptionalActions::Flush, &raw)?;
        Ok(Self {
            original: Some(original),
            keys: Self::read_keys(),
        })
    }

    /// Whether standard input is a terminal.
    pub(super) const fn is_interactive(&self) -> bool {
        self.original.is_some()
    }

    /// The screen's columns and rows; 80 by 24 when standard output is not a terminal.
    pub(super) fn size() -> (u16, u16) {
        termios::tcgetwinsize(io::stdout())
            .ok()
            .filter(|size| size.ws_col > 0 && size.ws_row > 0)
            .map_or(Self::FALLBACK, |size| (size.ws_col, size.ws_row))
    }

    /// The next chunk of what is typed; `None` once standard input ended.
    pub(super) async fn typed(&mut self) -> Option<Vec<u8>> {
        self.keys.recv().await
    }

    /// Reads standard input on a thread of its own, which ends with the input or with the process: a blocked
    /// read of a terminal cannot be cancelled.
    fn read_keys() -> mpsc::Receiver<Vec<u8>> {
        let (sender, receiver) = mpsc::channel(Self::BACKLOG);
        let spawned = std::thread::Builder::new()
            .name("igloo-stdin".to_owned())
            .spawn(move || {
                let mut stdin = io::stdin().lock();
                let mut buffer = [0_u8; Self::CHUNK];
                while let Ok(read) = stdin.read(&mut buffer) {
                    if read == 0 || sender.blocking_send(buffer[..read].to_vec()).is_err() {
                        break;
                    }
                }
            });
        // Without the thread, the receiver reports the input as ended at once.
        drop(spawned);
        receiver
    }

    /// Prints what the remote process printed.
    pub(super) fn print(bytes: &[u8]) -> io::Result<()> {
        let mut stdout = io::stdout().lock();
        stdout.write_all(bytes)?;
        stdout.flush()
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        if let Some(original) = &self.original {
            // Nothing can be done about a terminal that refuses its old mode.
            let _ = termios::tcsetattr(io::stdin(), OptionalActions::Flush, original);
        }
    }
}
