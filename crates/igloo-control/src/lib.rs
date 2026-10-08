//! The Igloo control plane: command bus, controllers, reactors, ports, adapters, inbound adapters,
//! and the CI module.
//!
//! The `igloo-control` binary is a thin `main.rs` over this library; tests build the same
//! platform in process.

pub mod adapters;
pub mod agents;
pub mod app;
pub mod ci;
mod composition;
mod config;
pub mod inbound;
pub mod platform;
pub mod ports;

#[cfg(test)]
mod testing;

pub use composition::{Server, ServerError};
pub use config::{Config, LogFormat};
