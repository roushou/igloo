//! The `igloo-worker` binary: reads `IGLOO_WORKER_*` configuration and runs the worker until
//! interrupted.

use eyre::WrapErr;
use igloo_worker::{Worker, WorkerConfig};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let config = WorkerConfig::from_env()
        .map_err(|errors| eyre::eyre!("invalid configuration: {errors}"))?;
    let worker = Worker::new(config).await.wrap_err("starting the worker")?;
    let cancel = CancellationToken::new();
    let shutdown = cancel.clone();
    let run = worker.run(cancel);
    tokio::pin!(run);
    tokio::select! {
        result = &mut run => return result.wrap_err("the worker stopped"),
        signal = tokio::signal::ctrl_c() => signal.wrap_err("waiting for ctrl-c")?,
    }
    shutdown.cancel();
    run.await.wrap_err("stopping the worker")
}
