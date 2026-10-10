//! Interactive terminals: what a client sends and receives over the terminal WebSocket of a
//! sandbox (`GET /v1/sandboxes/{id}/terminal`).
//!
//! The socket speaks the subprotocol [`TerminalRequest::PROTOCOL`]. Binary frames carry the
//! terminal's bytes in both directions: what the person types, and what the process prints.
//! Text frames carry the JSON control messages below, one per frame. The session ends when the
//! process exits (the server sends [`TerminalServerFrame::Exit`], then closes) or when either
//! side closes the socket (the process is killed).

use igloo_core::terminal::{TerminalFailure, TerminalOutcome, TerminalSize};
use igloo_core::{ValidationErrors, Validator};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The query of a terminal request: what to run and the screen to start with.
///
/// Invariant: `size` is a valid screen; `command` is the program and its arguments, empty for
/// the sandbox's default shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalRequest {
    /// The program and its arguments; empty runs `sh`.
    pub command: Vec<String>,
    /// The initial screen; 80x24 when the query names none.
    pub size: TerminalSize,
}

impl TerminalRequest {
    /// The WebSocket subprotocol the server selects.
    pub const PROTOCOL: &'static str = "igloo.terminal.v1";
    /// The prefix of the subprotocol a browser offers to carry its bearer token, since a
    /// browser cannot set headers on a WebSocket: `igloo.bearer.<token>`.
    pub const BEARER_PROTOCOL_PREFIX: &'static str = "igloo.bearer.";
}

impl TryFrom<Vec<(String, String)>> for TerminalRequest {
    type Error = ValidationErrors;

    fn try_from(params: Vec<(String, String)>) -> Result<Self, Self::Error> {
        let mut command = Vec::new();
        let default = TerminalSize::DEFAULT;
        let mut cols = Ok(u32::from(default.cols()));
        let mut rows = Ok(u32::from(default.rows()));
        let number = |value: &str| {
            value
                .parse::<u32>()
                .map_err(|_| "must be a number".to_owned())
        };
        for (key, value) in params {
            match key.as_str() {
                "command" => command.push(value),
                "cols" => cols = number(&value),
                "rows" => rows = number(&value),
                _ => {}
            }
        }
        let (cols, rows) = Validator::new()
            .field("cols", cols)
            .field("rows", rows)
            .finish()?;
        let (size,) = Validator::new()
            .field("size", TerminalSize::new(cols, rows))
            .finish()?;
        Ok(Self { command, size })
    }
}

/// A text frame the client sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TerminalClientFrame {
    /// The client's screen changed size; 1 to 65535 columns and rows.
    Resize {
        /// Columns.
        cols: u16,
        /// Rows.
        rows: u16,
    },
}

/// A text frame the server sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TerminalServerFrame {
    /// The terminal ended; the last frame before the server closes the socket. Exactly one of
    /// `code` and `failure` is set.
    Exit {
        /// The process's exit code (128 + signal when killed by a signal).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<i32>,
        /// Why the terminal ended without an exit code.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<TerminalEndReason>,
    },
}

/// Why a terminal ended without an exit code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TerminalEndReason {
    /// The sandbox is not running on its worker.
    SandboxUnavailable,
    /// The process or its pseudo-terminal could not be started.
    ExecutionError,
    /// The terminal was closed before the process ended.
    Closed,
    /// The worker's connection dropped, or the client read too slowly to keep up.
    Lost,
}

impl TerminalServerFrame {
    /// The frame for a terminal whose worker connection dropped or whose reader fell behind.
    #[must_use]
    pub const fn lost() -> Self {
        Self::Exit {
            code: None,
            failure: Some(TerminalEndReason::Lost),
        }
    }
}

impl From<TerminalOutcome> for TerminalServerFrame {
    fn from(outcome: TerminalOutcome) -> Self {
        match outcome {
            TerminalOutcome::Exited(code) => Self::Exit {
                code: Some(code),
                failure: None,
            },
            TerminalOutcome::Failed(failure) => Self::Exit {
                code: None,
                failure: Some(failure.into()),
            },
        }
    }
}

impl From<TerminalFailure> for TerminalEndReason {
    fn from(failure: TerminalFailure) -> Self {
        match failure {
            TerminalFailure::SandboxUnavailable => Self::SandboxUnavailable,
            TerminalFailure::ExecutionError => Self::ExecutionError,
            TerminalFailure::Closed => Self::Closed,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn query(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn a_bare_request_runs_the_default_shell_on_80_by_24() {
        let request = TerminalRequest::try_from(Vec::new()).expect("request");
        assert_eq!(request.command, Vec::<String>::new());
        assert_eq!(request.size, TerminalSize::DEFAULT);
    }

    #[test]
    fn the_query_names_the_command_and_the_screen() {
        let request = TerminalRequest::try_from(query(&[
            ("command", "bash"),
            ("command", "-l"),
            ("cols", "120"),
            ("rows", "40"),
        ]))
        .expect("request");
        assert_eq!(request.command, ["bash", "-l"]);
        assert_eq!(request.size, TerminalSize::new(120, 40).expect("size"));
    }

    #[test]
    fn every_bad_dimension_is_reported() {
        let errors = TerminalRequest::try_from(query(&[("cols", "wide"), ("rows", "tall")]))
            .expect_err("invalid");
        let fields: Vec<_> = errors.fields().map(|(field, _)| field).collect();
        assert_eq!(fields, ["cols", "rows"]);
        assert!(TerminalRequest::try_from(query(&[("cols", "0")])).is_err());
    }

    #[test]
    fn frames_have_a_stable_json_form() {
        let resize = TerminalClientFrame::Resize {
            cols: 100,
            rows: 30,
        };
        assert_eq!(
            serde_json::to_value(resize).expect("json"),
            json!({ "type": "resize", "cols": 100, "rows": 30 })
        );
        let exits = [
            TerminalServerFrame::from(TerminalOutcome::Exited(3)),
            TerminalServerFrame::from(TerminalOutcome::Failed(TerminalFailure::Closed)),
            TerminalServerFrame::lost(),
        ];
        assert_eq!(
            serde_json::to_value(exits).expect("json"),
            json!([
                { "type": "exit", "code": 3 },
                { "type": "exit", "failure": "closed" },
                { "type": "exit", "failure": "lost" },
            ])
        );
    }
}
