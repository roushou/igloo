//! The `igloo-control` binary: reads `IGLOO_*` configuration, serves until interrupted, then
//! shuts down gracefully.

use std::time::Duration;

use eyre::WrapErr;
use igloo_control::{Config, LogFormat, Server};
use tokio::signal::unix::{SignalKind, signal};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let config =
        Config::from_env().map_err(|errors| eyre::eyre!("invalid configuration: {errors}"))?;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    match config.log_format() {
        LogFormat::Pretty => tracing_subscriber::fmt().with_env_filter(filter).init(),
        LogFormat::Json => tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .init(),
    }
    let server = Server::start(&config)
        .await
        .wrap_err("starting the server")?;
    tracing::info!(
        rest = %server.rest_address(),
        gateway = %server.gateway_address(),
        data_dir = %config.data_dir().display(),
        public_url = config.public_url().map(tracing::field::display),
        embedded_worker = config.embedded_worker(),
        "igloo-control is serving"
    );
    let mut terminate = signal(SignalKind::terminate()).wrap_err("listening for SIGTERM")?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result.wrap_err("waiting for ctrl-c")?,
        _ = terminate.recv() => {}
    }
    tracing::info!("shutting down");
    server
        .shutdown(Duration::from_secs(30))
        .await
        .wrap_err("shutting down")
}
