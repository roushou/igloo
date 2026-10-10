//! Interactive terminals: the identity of a pseudo-terminal session and the size of its screen.

use std::fmt;
use std::num::NonZeroU16;

use crate::{Id, Prefixed};

/// Marker for [`TerminalId`]. A terminal is a live session on a worker, not a stored entity.
#[derive(Clone, Copy, Debug)]
pub struct Terminal;

impl Prefixed for Terminal {
    const PREFIX: &'static str = "term";
}

/// Identifies one terminal session; chosen by the server when it opens the terminal.
pub type TerminalId = Id<Terminal>;

/// The screen of a terminal in character cells.
///
/// Invariant: both dimensions are at least 1 and at most [`u16::MAX`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    cols: NonZeroU16,
    rows: NonZeroU16,
}

/// A terminal size with an empty or oversized dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a terminal is 1 to {max} columns and rows, not {cols}x{rows}", max = u16::MAX)]
pub struct TerminalSizeError {
    cols: u32,
    rows: u32,
}

impl TerminalSize {
    /// The conventional 80x24 screen.
    pub const DEFAULT: Self = Self {
        cols: NonZeroU16::MIN.saturating_add(79),
        rows: NonZeroU16::MIN.saturating_add(23),
    };

    /// A screen of `cols` by `rows` cells.
    pub fn new(cols: u32, rows: u32) -> Result<Self, TerminalSizeError> {
        let narrow = |value: u32| u16::try_from(value).ok().and_then(NonZeroU16::new);
        match (narrow(cols), narrow(rows)) {
            (Some(cols), Some(rows)) => Ok(Self { cols, rows }),
            _ => Err(TerminalSizeError { cols, rows }),
        }
    }

    /// Columns, at least 1.
    #[must_use]
    pub const fn cols(self) -> u16 {
        self.cols.get()
    }

    /// Rows, at least 1.
    #[must_use]
    pub const fn rows(self) -> u16 {
        self.rows.get()
    }
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl fmt::Display for TerminalSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.cols, self.rows)
    }
}

/// Why a terminal ended without its process reporting an exit code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalFailure {
    /// The sandbox is not running on the worker.
    SandboxUnavailable,
    /// The process or its pseudo-terminal could not be started.
    ExecutionError,
    /// The server closed the terminal before the process ended.
    Closed,
}

/// How a terminal ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalOutcome {
    /// The process exited with this code (128 + signal when killed by a signal).
    Exited(i32),
    /// The terminal ended without an exit code.
    Failed(TerminalFailure),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_needs_both_dimensions() {
        assert_eq!(TerminalSize::new(120, 40).map(TerminalSize::cols), Ok(120));
        assert!(TerminalSize::new(0, 24).is_err());
        assert!(TerminalSize::new(80, 0).is_err());
        assert!(TerminalSize::new(80, 70_000).is_err());
    }

    #[test]
    fn the_default_is_80_by_24() {
        let size = TerminalSize::default();
        assert_eq!((size.cols(), size.rows()), (80, 24));
        assert_eq!(size.to_string(), "80x24");
    }

    #[test]
    fn ids_carry_the_term_prefix() {
        let id: TerminalId = Id::from_uuid(uuid::Uuid::from_u128(1));
        assert!(id.to_string().starts_with("term_"));
        assert_eq!(id.to_string().parse::<TerminalId>(), Ok(id));
    }
}
