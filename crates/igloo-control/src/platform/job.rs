use std::sync::Arc;

use async_trait::async_trait;
use igloo_core::job::{
    FencingToken, Job, JobAction, JobError, JobFailure, JobId, JobPhase, JobSpec, Lease,
};
use igloo_core::sandbox::{Sandbox, SandboxEvent, SandboxId};
use igloo_core::worker::WorkerId;
use igloo_core::{
    Actor, Entity, ErrorCode, Resource, SystemComponent, Timestamp, ValidationErrors,
};
use jiff::SignedDuration;

use crate::app::{
    AppError, Command, CommandBus, CommandHandler, ControllerSettings, EntityHandler, Extension,
    InstallError, PlatformBuilder, Reactor, Reconciler, RequestContext,
};
use crate::ports::{
    Clock, EntityStore, EventEnvelope, IdGenerator, IdGeneratorExt, SecretStore, StorageError,
    Versioned,
};

/// Submits a job to run in an existing sandbox.
pub struct SubmitJob {
    /// What to run.
    pub spec: JobSpec,
}

/// Leases a job to the worker holding its sandbox.
pub struct LeaseJob {
    /// The job.
    pub job: JobId,
    /// The claiming worker.
    pub worker: WorkerId,
    /// How long the lease holds unless renewed.
    pub duration: SignedDuration,
}

/// Extends a live lease.
pub struct RenewJobLease {
    /// The job.
    pub job: JobId,
    /// The lease's token.
    pub token: FencingToken,
    /// The new duration from now.
    pub duration: SignedDuration,
}

/// The lease holder started the job.
pub struct StartJob {
    /// The job.
    pub job: JobId,
    /// The lease's token.
    pub token: FencingToken,
}

/// The lease holder reports the exit code.
pub struct FinishJob {
    /// The job.
    pub job: JobId,
    /// The lease's token.
    pub token: FencingToken,
    /// The exit code.
    pub exit_code: i32,
}

/// The lease holder reports a failure.
pub struct FailJob {
    /// The job.
    pub job: JobId,
    /// The lease's token.
    pub token: FencingToken,
    /// Why.
    pub reason: JobFailure,
}

/// Cancels a job.
pub struct CancelJob {
    /// The job.
    pub job: JobId,
}

/// Fails a job whose lease expired; does nothing while its lease is live.
pub struct FailLostLease {
    /// The job.
    pub job: JobId,
}

impl Command for SubmitJob {
    type Output = JobId;
    const NAME: &'static str = "job.submit";
}

impl Command for LeaseJob {
    type Output = Lease;
    const NAME: &'static str = "job.lease";
}

impl Command for RenewJobLease {
    type Output = ();
    const NAME: &'static str = "job.renew";
}

impl Command for StartJob {
    type Output = ();
    const NAME: &'static str = "job.start";
}

impl Command for FinishJob {
    type Output = ();
    const NAME: &'static str = "job.finish";
}

impl Command for FailJob {
    type Output = ();
    const NAME: &'static str = "job.fail";
}

impl Command for CancelJob {
    type Output = ();
    const NAME: &'static str = "job.cancel";
}

impl Command for FailLostLease {
    type Output = ();
    const NAME: &'static str = "job.fail_lost_lease";
}

/// Why a job cannot be submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SubmitJobError {
    /// The job's sandbox has stopped or failed.
    #[error("the sandbox has ended")]
    SandboxEnded,
}

impl ErrorCode for SubmitJobError {
    fn code(&self) -> &'static str {
        match self {
            Self::SandboxEnded => "job.sandbox_ended",
        }
    }
}

/// Submits a job after checking its sandbox exists and has not ended, and that the sandbox's
/// repository has every secret the job names.
struct SubmitJobHandler {
    jobs: Arc<dyn EntityStore<Job>>,
    sandboxes: Arc<dyn EntityStore<Sandbox>>,
    secrets: Arc<dyn SecretStore>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
}

#[async_trait]
impl CommandHandler<SubmitJob> for SubmitJobHandler {
    async fn handle(
        &self,
        command: SubmitJob,
        context: &RequestContext,
    ) -> Result<JobId, AppError> {
        let sandbox_id = command.spec.sandbox();
        let sandbox = self
            .sandboxes
            .load(sandbox_id)
            .await?
            .ok_or_else(|| AppError::not_found(Sandbox::NAME, &sandbox_id))?;
        if sandbox.entity().status().phase().is_terminal() {
            return Err(AppError::domain(&SubmitJobError::SandboxEnded));
        }
        let names = command.spec.secrets();
        if !names.is_empty() {
            let Some(repo) = sandbox.entity().spec().repo() else {
                return Err(
                    ValidationErrors::single("secrets", "the sandbox has no repository").into(),
                );
            };
            let set = self.secrets.names(repo).await?;
            let mut missing = ValidationErrors::default();
            for name in names.iter().filter(|name| !set.contains(name)) {
                missing.add(format!("secrets.{name}"), "not set");
            }
            missing.into_result(())?;
        }
        let id = self.ids.next::<Job>();
        let now = self.clock.now();
        let mut job = Versioned::new(Job::new(id, command.spec, now));
        self.jobs
            .commit(&mut job, &context.commit_meta(now))
            .await?;
        Ok(id)
    }
}

/// Read access to jobs beyond loading one by id.
#[derive(Clone)]
pub struct JobQueries {
    jobs: Arc<dyn EntityStore<Job>>,
    sandboxes: Arc<dyn EntityStore<Sandbox>>,
}

impl JobQueries {
    /// Queries over the job and sandbox stores.
    #[must_use]
    pub fn new(jobs: Arc<dyn EntityStore<Job>>, sandboxes: Arc<dyn EntityStore<Sandbox>>) -> Self {
        Self { jobs, sandboxes }
    }

    /// Queued jobs whose sandbox is assigned to `worker`, oldest first: the jobs that worker
    /// should lease.
    pub async fn queued_for(&self, worker: WorkerId) -> Result<Vec<Job>, StorageError> {
        let on_worker: Vec<_> = self
            .sandboxes
            .all()
            .await?
            .into_iter()
            .filter(|sandbox| sandbox.status().worker() == Some(worker))
            .map(|sandbox| sandbox.id())
            .collect();
        let mut queued: Vec<Job> = self
            .jobs
            .all()
            .await?
            .into_iter()
            .filter(|job| {
                job.phase() == JobPhase::Queued && on_worker.contains(&job.spec().sandbox())
            })
            .collect();
        queued.sort_by_key(Entity::id);
        Ok(queued)
    }

    /// Jobs whose live lease `worker` holds, oldest first: what it should be running.
    pub async fn leased_to(
        &self,
        worker: WorkerId,
        now: Timestamp,
    ) -> Result<Vec<Job>, StorageError> {
        let mut leased: Vec<Job> = self
            .jobs
            .all()
            .await?
            .into_iter()
            .filter(|job| {
                matches!(job.phase(), JobPhase::Leased | JobPhase::Running)
                    && job
                        .current_lease()
                        .is_some_and(|lease| lease.worker() == worker && lease.is_live(now))
            })
            .collect();
        leased.sort_by_key(Entity::id);
        Ok(leased)
    }

    /// Unfinished jobs running in `sandbox`.
    async fn unfinished_in(&self, sandbox: SandboxId) -> Result<Vec<Job>, StorageError> {
        Ok(self
            .jobs
            .all()
            .await?
            .into_iter()
            .filter(|job| job.spec().sandbox() == sandbox && !job.phase().is_terminal())
            .collect())
    }
}

/// Cancels the unfinished jobs of a sandbox once it stops or fails.
struct CancelJobsOfEndedSandboxes {
    queries: JobQueries,
}

#[async_trait]
impl Reactor for CancelJobsOfEndedSandboxes {
    fn name(&self) -> &'static str {
        "job.cancel_on_sandbox_end"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let Some(sandbox) = event.subject_as::<Sandbox>() else {
            return Ok(());
        };
        let ended = matches!(
            event.decode::<SandboxEvent>()?,
            SandboxEvent::StatusRecorded { phase, .. } if phase.is_terminal()
        );
        if !ended {
            return Ok(());
        }
        let actor = Actor::System {
            component: SystemComponent::Reactor,
        };
        for job in self.queries.unfinished_in(sandbox).await? {
            bus.dispatch(
                CancelJob { job: job.id() },
                RequestContext::caused_by(event, actor),
            )
            .await?;
        }
        Ok(())
    }
}

/// Fails jobs whose lease expired.
struct LeaseExpiry {
    bus: CommandBus,
    ids: Arc<dyn IdGenerator>,
}

impl Reconciler<Job> for LeaseExpiry {
    async fn reconcile(&self, job: &Job, actions: Vec<JobAction>) -> Result<(), AppError> {
        for action in actions {
            match action {
                JobAction::FailLeaseLost => {
                    let context = RequestContext::new(
                        Actor::System {
                            component: SystemComponent::Controller,
                        },
                        self.ids.next(),
                    );
                    self.bus
                        .dispatch(FailLostLease { job: job.id() }, context)
                        .await?;
                }
            }
        }
        Ok(())
    }
}

/// Jobs: submission, leasing with fencing tokens, results, failure when a lease expires, and
/// cancellation when their sandbox ends.
pub struct JobModule {
    /// Pacing of the lease expiry controller.
    pub settings: ControllerSettings,
}

impl Extension for JobModule {
    fn name(&self) -> &'static str {
        "job"
    }

    fn install(self: Box<Self>, platform: &mut PlatformBuilder) -> Result<(), InstallError> {
        let jobs = platform.store::<Job>()?;
        let sandboxes = platform.store::<Sandbox>()?;
        let ports = platform.ports().clone();
        let clock = &ports.clock;
        platform.command(SubmitJobHandler {
            jobs: Arc::clone(&jobs),
            sandboxes: Arc::clone(&sandboxes),
            secrets: Arc::clone(&ports.secrets),
            clock: Arc::clone(clock),
            ids: Arc::clone(&ports.ids),
        })?;
        platform.command(EntityHandler::new(
            Arc::clone(&jobs),
            Arc::clone(clock),
            |command: &LeaseJob| command.job,
            |job: &mut Job, command: &LeaseJob, now| -> Result<Lease, JobError> {
                job.lease(command.worker, command.duration, now)
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(&jobs),
            Arc::clone(clock),
            |command: &RenewJobLease| command.job,
            |job: &mut Job, command: &RenewJobLease, now| -> Result<(), JobError> {
                job.renew(command.token, command.duration, now)
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(&jobs),
            Arc::clone(clock),
            |command: &StartJob| command.job,
            |job: &mut Job, command: &StartJob, _| -> Result<(), JobError> {
                job.start(command.token)
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(&jobs),
            Arc::clone(clock),
            |command: &FinishJob| command.job,
            |job: &mut Job, command: &FinishJob, _| -> Result<(), JobError> {
                job.finish(command.token, command.exit_code)
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(&jobs),
            Arc::clone(clock),
            |command: &FailJob| command.job,
            |job: &mut Job, command: &FailJob, _| -> Result<(), JobError> {
                job.fail(command.token, command.reason)
            },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(&jobs),
            Arc::clone(clock),
            |command: &CancelJob| command.job,
            |job: &mut Job, _: &CancelJob, _| job.cancel(),
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(&jobs),
            Arc::clone(clock),
            |command: &FailLostLease| command.job,
            |job: &mut Job, _: &FailLostLease, now| job.lose_lease(now),
        ))?;
        let expiry = LeaseExpiry {
            bus: platform.bus(),
            ids: Arc::clone(&ports.ids),
        };
        platform.controller(expiry, self.settings)?;
        platform.reactor(CancelJobsOfEndedSandboxes {
            queries: JobQueries::new(jobs, sandboxes),
        });
        Ok(())
    }
}
