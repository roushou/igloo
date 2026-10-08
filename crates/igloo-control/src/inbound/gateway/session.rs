use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use igloo_core::job::{FencingToken, Job, JobFailure, JobId, JobPhase, Lease};
use igloo_core::process::OutputStream;
use igloo_core::sandbox::{Sandbox, SandboxId, SandboxPhase};
use igloo_core::seal::{SealFailure, SealId};
use igloo_core::snapshot::{SnapshotId, SnapshotManifest};
use igloo_core::worker::WorkerId;
use igloo_core::{Entity, ErrorCode, Generation, Resource};
use igloo_worker_protocol::v1;
use igloo_worker_protocol::v1::connect_request::Message as Inbound;
use igloo_worker_protocol::v1::connect_response::Message as Outbound;
use igloo_worker_protocol::v1::job_result::Outcome;
use jiff::SignedDuration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tonic::{Status, Streaming};

use super::Shared;
use super::secrets::JobSecrets;
use crate::app::AppError;
use crate::platform::{
    DisconnectWorker, FailJob, FailSeal, FinishJob, LeaseJob, RecordSandboxStatus, RenewJobLease,
    StartJob,
};
use crate::ports::SecretValue;

/// One worker's connection: what was last sent to it and which leases it was granted.
pub(super) struct Session {
    worker: WorkerId,
    shared: Arc<Shared>,
    outbound: mpsc::Sender<Result<v1::ConnectResponse, Status>>,
    assignment: Option<v1::Assignment>,
    granted: HashMap<JobId, FencingToken>,
    manifests: HashMap<SnapshotId, SnapshotManifest>,
    /// Seals sent during this session; a new session sends the pending ones again.
    seals: HashSet<SealId>,
    /// The secret values of granted jobs, masked out of their output.
    masks: HashMap<JobId, Vec<SecretValue>>,
}

impl Session {
    pub(super) fn new(
        worker: WorkerId,
        shared: Arc<Shared>,
        outbound: mpsc::Sender<Result<v1::ConnectResponse, Status>>,
    ) -> Self {
        Self {
            worker,
            shared,
            outbound,
            assignment: None,
            granted: HashMap::new(),
            manifests: HashMap::new(),
            seals: HashSet::new(),
            masks: HashMap::new(),
        }
    }

    /// Serves the worker until it disconnects or the platform shuts down, then records the
    /// disconnection.
    pub(super) async fn run(
        mut self,
        mut inbound: Streaming<v1::ConnectRequest>,
        cancel: CancellationToken,
    ) -> Result<(), AppError> {
        let mut head = self.shared.events.head();
        head.borrow_and_update();
        self.regrant().await;
        self.sync().await;
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                message = inbound.message() => match message {
                    Ok(Some(message)) => self.handle(message).await,
                    Ok(None) | Err(_) => break,
                },
                changed = head.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    self.sync().await;
                }
            }
            if self.outbound.is_closed() {
                break;
            }
        }
        let command = DisconnectWorker {
            worker: self.worker,
        };
        self.shared
            .bus
            .dispatch(command, self.shared.context())
            .await
    }

    async fn send(&self, message: Outbound) {
        let response = v1::ConnectResponse {
            message: Some(message),
        };
        // A closed channel means the worker left; the loop notices and ends the session.
        let _ = self.outbound.send(Ok(response)).await;
    }

    /// Pushes what changed since the last sync: the assignment, new leases, cancellations.
    async fn sync(&mut self) {
        if let Err(error) = self.try_sync().await {
            tracing::warn!(worker = %self.worker, %error, "gateway sync failed");
        }
    }

    /// The layers of `snapshot` with download URLs; none if the snapshot is unknown, which the
    /// worker reports as an unavailable snapshot.
    /// `sandbox`'s assignment: its snapshot's layers with download URLs, and its environment
    /// over the snapshot's. A sandbox whose snapshot is unknown gets no layers, which the
    /// worker reports as an unavailable snapshot.
    async fn assigned(&mut self, sandbox: &Sandbox) -> Result<v1::AssignedSandbox, AppError> {
        let mut assigned = v1::AssignedSandbox::from(sandbox);
        let snapshot = sandbox.spec().snapshot();
        let manifest = match self.manifests.entry(snapshot) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => match self.shared.snapshots.manifest(snapshot).await? {
                Some(manifest) => entry.insert(manifest),
                None => return Ok(assigned),
            },
        };
        for (key, value) in manifest.env().iter() {
            assigned
                .env
                .entry(key.to_owned())
                .or_insert_with(|| value.to_owned());
        }
        let layers = manifest
            .layers()
            .iter()
            .map(|layer| {
                let url = self.shared.blob_urls.download(layer.digest());
                v1::SnapshotLayer::new(layer, url)
            })
            .collect();
        Ok(assigned.with_layers(layers))
    }

    async fn try_sync(&mut self) -> Result<(), AppError> {
        let mut sandboxes = self.shared.sandboxes.on_worker(self.worker).await?;
        sandboxes.sort_by_key(Entity::id);
        let mut assigned = Vec::with_capacity(sandboxes.len());
        for sandbox in &sandboxes {
            assigned.push(self.assigned(sandbox).await?);
        }
        let assignment = v1::Assignment {
            sandboxes: assigned,
        };
        if self.assignment.as_ref() != Some(&assignment) {
            self.send(Outbound::Assignment(assignment.clone())).await;
            self.assignment = Some(assignment);
        }

        // Only jobs whose sandbox is in the assignment the worker was sent: a sandbox scheduled
        // since then reaches the worker with the next sync, before its jobs.
        let assigned: Vec<_> = sandboxes.iter().map(Entity::id).collect();
        self.send_seals(&assigned).await?;
        let queued = self.shared.jobs.queued_for(self.worker).await?;
        for job in queued
            .into_iter()
            .filter(|job| assigned.contains(&job.spec().sandbox()))
        {
            let command = LeaseJob {
                job: job.id(),
                worker: self.worker,
                duration: self.lease_duration(),
            };
            match self
                .shared
                .bus
                .dispatch(command, self.shared.context())
                .await
            {
                Ok(lease) => self.grant(&job, &lease).await,
                Err(error) => tracing::warn!(job = %job.id(), %error, "leasing failed"),
            }
        }

        for (job, token) in self.granted.clone() {
            let Some(stored) = self.shared.job_store.load(job).await? else {
                continue;
            };
            match stored.entity().phase() {
                JobPhase::Cancelled => {
                    self.granted.remove(&job);
                    self.cancel(job, token).await;
                }
                phase if phase.is_terminal() => {
                    self.granted.remove(&job);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Sends again the leases the worker still holds, for a worker that reconnected after
    /// losing its state.
    async fn regrant(&mut self) {
        let now = self.shared.clock.now();
        let leased = match self.shared.jobs.leased_to(self.worker, now).await {
            Ok(leased) => leased,
            Err(error) => {
                tracing::warn!(worker = %self.worker, %error, "listing leases failed");
                return;
            }
        };
        for job in leased {
            if let Some(lease) = job.current_lease() {
                self.grant(&job, &lease.clone()).await;
            }
        }
    }

    /// Sends the grant of `lease` with the job's secrets in its environment. A job whose
    /// secrets cannot be resolved fails instead.
    async fn grant(&mut self, job: &Job, lease: &Lease) {
        let mut grant = v1::LeaseGrant::new(job, lease, self.lease_seconds());
        match self.shared.secrets.resolve(job).await {
            Ok(secrets) => {
                let mut values = Vec::with_capacity(secrets.len());
                for (name, value) in secrets {
                    grant
                        .env
                        .insert(name.to_string(), value.expose().to_owned());
                    values.push(value);
                }
                self.masks.insert(job.id(), values);
                self.granted.insert(job.id(), lease.token());
                self.send(Outbound::LeaseGrant(grant)).await;
            }
            Err(error) => {
                tracing::warn!(job = %job.id(), %error, "resolving the job's secrets failed");
                let command = FailJob {
                    job: job.id(),
                    token: lease.token(),
                    reason: JobFailure::ExecutionError,
                };
                if let Err(error) = self
                    .shared
                    .bus
                    .dispatch(command, self.shared.context())
                    .await
                {
                    tracing::warn!(job = %job.id(), %error, "failing the job failed");
                }
            }
        }
    }

    async fn handle(&mut self, request: v1::ConnectRequest) {
        let result = match request.message {
            Some(Inbound::Heartbeat(heartbeat)) => {
                self.renew(heartbeat).await;
                Ok(())
            }
            Some(Inbound::SandboxStatus(status)) => self.record_status(&status).await,
            Some(Inbound::JobStarted(started)) => self.start(&started).await,
            Some(Inbound::LogChunk(chunk)) => self.append_log(chunk).await,
            Some(Inbound::JobResult(result)) => self.complete(result).await,
            Some(Inbound::SealFailed(failed)) => self.fail_seal(&failed).await,
            Some(Inbound::Hello(_)) | None => Ok(()),
        };
        if let Err(error) = result {
            tracing::warn!(worker = %self.worker, %error, cause = ?error, "worker message rejected");
        }
    }

    async fn renew(&mut self, heartbeat: v1::Heartbeat) {
        for held in heartbeat.leases {
            let Ok(job) = held.job_id.parse::<JobId>() else {
                continue;
            };
            let token = FencingToken::from(held.token);
            let command = RenewJobLease {
                job,
                token,
                duration: self.lease_duration(),
            };
            match self
                .shared
                .bus
                .dispatch(command, self.shared.context())
                .await
            {
                Ok(()) => {}
                // The next heartbeat renews it; the lease outlives several heartbeats.
                Err(error) if error.is_transient() => {
                    tracing::warn!(%job, %error, "renewing a lease failed");
                }
                Err(_) => {
                    self.granted.remove(&job);
                    self.cancel(job, token).await;
                }
            }
        }
    }

    async fn record_status(&self, status: &v1::SandboxStatus) -> Result<(), AppError> {
        let sandbox: SandboxId = parse(&status.sandbox_id)?;
        let phase = SandboxPhase::try_from(status).map_err(invalid)?;
        let observed_generation =
            Generation::try_from(status.observed_generation).map_err(invalid)?;
        let command = RecordSandboxStatus {
            sandbox,
            phase,
            observed_generation,
        };
        self.shared
            .bus
            .dispatch(command, self.shared.context())
            .await
    }

    async fn start(&self, started: &v1::JobStarted) -> Result<(), AppError> {
        let command = StartJob {
            job: parse(&started.job_id)?,
            token: FencingToken::from(started.token),
        };
        self.shared
            .bus
            .dispatch(command, self.shared.context())
            .await
    }

    /// Stores output only from the current lease holder.
    /// Stores output only from the current lease holder, with the job's secrets masked.
    async fn append_log(&mut self, chunk: v1::LogChunk) -> Result<(), AppError> {
        let job: JobId = parse(&chunk.job_id)?;
        let stream = OutputStream::try_from(chunk.stream()).map_err(invalid)?;
        let Some(stored) = self.shared.job_store.load(job).await? else {
            return Ok(());
        };
        let current = stored.entity().current_lease().map(Lease::token);
        if current != Some(FencingToken::from(chunk.token)) {
            return Ok(());
        }
        if !self.masks.contains_key(&job) {
            let values = self
                .shared
                .secrets
                .resolve(stored.entity())
                .await?
                .into_iter()
                .map(|(_, value)| value)
                .collect();
            self.masks.insert(job, values);
        }
        let values = self.masks.get(&job).map_or(&[][..], Vec::as_slice);
        let data = JobSecrets::mask(&chunk.data, values);
        self.shared
            .logs
            .append(job, stream, chunk.offset, &data)
            .await
            .map_err(AppError::from)
    }

    /// Records a result and acknowledges it, accepted or refused as stale. Infrastructure
    /// failures are not acknowledged, so the worker resends.
    async fn fail_seal(&self, failed: &v1::SealFailed) -> Result<(), AppError> {
        tracing::warn!(
            seal = failed.seal_id,
            reason = failed.reason,
            "the worker could not seal"
        );
        let command = FailSeal {
            seal: parse(&failed.seal_id)?,
            reason: SealFailure::WorkerError,
        };
        self.shared
            .bus
            .dispatch(command, self.shared.context())
            .await
    }

    /// Sends the pending seals of `sandboxes` not sent yet in this session.
    async fn send_seals(&mut self, sandboxes: &[SandboxId]) -> Result<(), AppError> {
        for seal in self.shared.seals.pending_in(sandboxes).await? {
            if self.seals.insert(seal.id()) {
                let request = v1::SealRequest {
                    seal_id: seal.id().to_string(),
                    sandbox_id: seal.sandbox().to_string(),
                    upload_url: self.shared.blob_urls.seal_upload(seal.id()).to_string(),
                };
                self.send(Outbound::SealRequest(request)).await;
            }
        }
        Ok(())
    }

    async fn complete(&mut self, result: v1::JobResult) -> Result<(), AppError> {
        let job: JobId = parse(&result.job_id)?;
        let token = FencingToken::from(result.token);
        let outcome = match result.outcome {
            Some(Outcome::ExitCode(exit_code)) => {
                let command = FinishJob {
                    job,
                    token,
                    exit_code,
                };
                self.shared
                    .bus
                    .dispatch(command, self.shared.context())
                    .await
            }
            Some(Outcome::Failure(failure)) => {
                let reason = v1::JobFailure::try_from(failure)
                    .map_err(|_| invalid("failure"))
                    .and_then(|failure| JobFailure::try_from(failure).map_err(invalid))?;
                let command = FailJob { job, token, reason };
                self.shared
                    .bus
                    .dispatch(command, self.shared.context())
                    .await
            }
            None => return Err(invalid("outcome")),
        };
        let (accepted, reason) = match &outcome {
            Ok(()) => (true, String::new()),
            Err(error @ AppError::Domain { .. }) => (false, error.code().to_owned()),
            Err(_) => return outcome,
        };
        self.granted.remove(&job);
        let ack = v1::ResultAck {
            job_id: job.to_string(),
            token: token.get(),
            accepted,
            reason,
        };
        self.send(Outbound::ResultAck(ack)).await;
        Ok(())
    }

    async fn cancel(&self, job: JobId, token: FencingToken) {
        let cancel = v1::CancelJob {
            job_id: job.to_string(),
            token: token.get(),
        };
        self.send(Outbound::CancelJob(cancel)).await;
    }

    fn lease_duration(&self) -> SignedDuration {
        SignedDuration::try_from(self.shared.lease_ttl).unwrap_or(SignedDuration::from_secs(30))
    }

    fn lease_seconds(&self) -> u32 {
        u32::try_from(self.shared.lease_ttl.as_secs()).unwrap_or(u32::MAX)
    }
}

/// An id field of a worker message.
fn parse<T: std::str::FromStr>(id: &str) -> Result<T, AppError> {
    id.parse().map_err(|_| invalid("id"))
}

/// A field of a worker message that does not decode.
fn invalid(field: impl std::fmt::Display) -> AppError {
    AppError::Validation(igloo_core::ValidationErrors::single(
        "message",
        field.to_string(),
    ))
}
