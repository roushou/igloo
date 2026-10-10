//! Inbound adapters: the REST API, the MCP server and the worker gateway. They parse requests,
//! dispatch commands or run queries, and map results. No domain logic lives here.

mod blob_urls;
pub mod gateway;
mod git;
pub mod mcp;
pub mod rest;
mod terminals;
mod web;

pub use blob_urls::{
    BlobAccess, BlobSignature, BlobSigningKey, BlobUrlRejected, BlobUrls, WeakSecret,
};
pub use git::{GitHttp, GitHttpError};
pub(crate) use terminals::WorkerLink;
pub use terminals::{TerminalError, TerminalEvent, TerminalHub, TerminalLaunch, TerminalSession};
pub use web::WebConsole;
