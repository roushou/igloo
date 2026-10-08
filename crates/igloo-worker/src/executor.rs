use std::sync::Arc;
use std::time::Duration;

use igloo_core::job::JobFailure;
use igloo_worker_protocol::v1;
use igloo_worker_protocol::v1::connect_request::Message as Uplink;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::runtime::{ExitOutcome, LocalSandbox, OutputChunk, Process, SandboxRuntime};

/// One leased job running in a local sandbox.
pub(crate) struct Execution {
    pub(crate) grant: v1::LeaseGrant,
    pub(crate) sandbox: LocalSandbox,
    pub(crate) runtime: Arc<dyn SandboxRuntime>,
    pub(crate) uplink: mpsc::Sender<Uplink>,
    pub(crate) cancel: CancellationToken,
}

impl Execution {
    /// Runs the job, streaming its output. Returns its result, or `None` when it was cancelled:
    /// the server already ended the job.
    pub(crate) async fn run(self) -> Option<v1::JobResult> {
        let Self {
            grant,
            sandbox,
            runtime,
            uplink,
            cancel,
        } = self;
        let started = v1::JobStarted {
            job_id: grant.job_id.clone(),
            token: grant.token,
        };
        let _ = uplink.send(Uplink::JobStarted(started)).await;
        let process = Process {
            argv: grant.argv.clone(),
            env: grant.env.clone().into_iter().collect(),
            timeout: Duration::from_secs(u64::from(grant.timeout_seconds)),
        };
        let (chunks, mut received) = mpsc::channel::<OutputChunk>(64);
        let forward = async {
            while let Some(chunk) = received.recv().await {
                let mut message = v1::LogChunk {
                    job_id: grant.job_id.clone(),
                    token: grant.token,
                    offset: chunk.offset,
                    data: chunk.data,
                    ..v1::LogChunk::default()
                };
                message.set_stream(chunk.stream.into());
                let _ = uplink.send(Uplink::LogChunk(message)).await;
            }
        };
        let (outcome, ()) = tokio::join!(runtime.exec(&sandbox, process, chunks, cancel), forward);
        let outcome = match outcome {
            Ok(ExitOutcome::Exited(code)) => v1::job_result::Outcome::ExitCode(code),
            Ok(ExitOutcome::TimedOut) => {
                v1::job_result::Outcome::Failure(v1::JobFailure::from(JobFailure::TimedOut).into())
            }
            Ok(ExitOutcome::Cancelled) => return None,
            Err(error) => {
                tracing::warn!(job = grant.job_id, %error, "the job could not run");
                v1::job_result::Outcome::Failure(
                    v1::JobFailure::from(JobFailure::ExecutionError).into(),
                )
            }
        };
        Some(v1::JobResult {
            job_id: grant.job_id,
            token: grant.token,
            outcome: Some(outcome),
        })
    }
}
