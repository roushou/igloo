//! The Igloo Rust SDK.
//!
//! ```no_run
//! # async fn example() -> Result<(), igloo::Error> {
//! use igloo::{Client, Run};
//!
//! let client = Client::new("http://127.0.0.1:7000", "dev-token")?;
//! let mut running = client
//!     .run(std::path::Path::new("."), Run::new(vec!["cargo".into(), "test".into()]))
//!     .await?;
//! while let Some(event) = running.logs().next().await {
//!     println!("{:?}", event?);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Depends only on `igloo-api`, an HTTP client and a WebSocket client. Its public API is a stability promise: small,
//! documented, and free of CLI concerns.

mod archive;
mod client;
mod logs;
mod run;
mod terminal;
mod workspaces;

pub use archive::Layer;
pub use client::{Client, Error};
pub use igloo_api::run as ci;
pub use igloo_api::terminal::{TerminalEndReason, TerminalMode};
pub use igloo_api::{change, job, problem, repo, sandbox, seal, snapshot, task, workspace};
pub use logs::{LogEvent, LogStream, OutputStream};
pub use run::{Run, Running};
pub use terminal::{Terminal, TerminalEvent, TerminalExit, TerminalInput, TerminalOutput};
