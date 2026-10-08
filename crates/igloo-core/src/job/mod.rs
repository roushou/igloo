//! Jobs: units of work claimed by a worker through a lease and reported with its fencing token.

mod lease;
mod spec;

use jiff::SignedDuration;
use serde::{Deserialize, Serialize};

use crate::worker::WorkerId;
use crate::{
    Entity, ErrorCode, Event, Generation, Id, Labels, Plan, Prefixed, Resource, Timestamp,
};

pub use lease::{FencingToken, Lease};
pub use spec::{JobSpec, JobTimeout, TimeoutOutOfRange};

/// Identifies a job (`job_...`).
pub type JobId = Id<Job>;

/// A job and its current lease.
///
/// Invariant: at most one live lease; the lease's token is the greatest ever issued for the job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    id: JobId,
    spec: JobSpec,
    phase: JobPhase,
    lease: Option<Lease>,
    submitted_at: Timestamp,
    events: Vec<JobEvent>,
}

/// Where a job is in its lifecycle.
///
/// Invariant: `Finished`, `Failed` and `Cancelled` are terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum JobPhase {
    /// Waiting for a worker to lease it.
    Queued,
    /// Leased, not started yet.
    Leased,
    /// Running on the lease holder.
    Running,
    /// The process exited. Terminal.
    Finished {
        /// The process exit code; non-zero is still a finished job.
        exit_code: i32,
    },
    /// The job could not complete. Terminal.
    Failed {
        /// Why.
        reason: JobFailure,
    },
    /// Cancelled before completion. Terminal.
    Cancelled,
}

/// Why a job failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobFailure {
    /// The process exceeded its timeout.
    TimedOut,
    /// The job's sandbox was not available on the worker.
    SandboxUnavailable,
    /// The process could not be started.
    ExecutionError,
    /// The lease expired before the holder reported a result. The job may have had effects;
    /// running it again is the submitter's decision.
    LeaseLost,
}

/// How a job ended, from its terminal event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ending", rename_all = "snake_case")]
pub enum JobEnding {
    /// The process exited with `code`.
    Exited {
        /// The exit code.
        code: i32,
    },
    /// The job could not complete.
    Failed {
        /// Why.
        reason: JobFailure,
    },
    /// The job was cancelled.
    Cancelled,
}

impl JobEnding {
    /// Whether the process exited 0.
    #[must_use]
    pub const fn succeeded(&self) -> bool {
        matches!(self, Self::Exited { code: 0 })
    }
}

impl std::fmt::Display for JobEnding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exited { code } => write!(f, "exited with {code}"),
            Self::Failed { reason } => write!(f, "did not complete: {reason}"),
            Self::Cancelled => f.write_str("was cancelled"),
        }
    }
}

impl std::fmt::Display for JobFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TimedOut => "timed out",
            Self::SandboxUnavailable => "sandbox unavailable",
            Self::ExecutionError => "could not start",
            Self::LeaseLost => "lease lost",
        })
    }
}

impl JobEvent {
    /// How the job ended, if this event ends it.
    #[must_use]
    pub const fn ending(&self) -> Option<JobEnding> {
        match self {
            Self::Finished { exit_code } => Some(JobEnding::Exited { code: *exit_code }),
            Self::Failed { reason } => Some(JobEnding::Failed { reason: *reason }),
            Self::Cancelled => Some(JobEnding::Cancelled),
            Self::Submitted { .. }
            | Self::Leased { .. }
            | Self::LeaseRenewed { .. }
            | Self::Started => None,
        }
    }
}

/// What a job's controller may be asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobAction {
    /// Fail the job: its lease expired while it was leased or running.
    FailLeaseLost,
}

/// Facts about a job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobEvent {
    /// The job was submitted.
    Submitted {
        /// Its id.
        id: JobId,
        /// What to do.
        spec: JobSpec,
        /// When.
        at: Timestamp,
    },
    /// A worker leased the job.
    Leased {
        /// The new lease.
        lease: Lease,
    },
    /// The live lease was extended.
    LeaseRenewed {
        /// The new expiry.
        expires_at: Timestamp,
    },
    /// The job started.
    Started,
    /// The process exited.
    Finished {
        /// The exit code.
        exit_code: i32,
    },
    /// The job failed.
    Failed {
        /// Why.
        reason: JobFailure,
    },
    /// The job was cancelled.
    Cancelled,
}

/// Why a job change is rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobError {
    /// The change carries a token that is not the current lease's, or the lease expired.
    #[error("stale lease token {presented}")]
    StaleLease {
        /// The token presented.
        presented: FencingToken,
    },
    /// The change does not apply in the current phase.
    #[error("cannot {change} a job that is {from}")]
    InvalidTransition {
        /// The current phase.
        from: &'static str,
        /// The rejected change.
        change: &'static str,
    },
    /// A lease or renewal duration that is not positive.
    #[error("lease duration must be positive")]
    InvalidLeaseDuration,
}

impl ErrorCode for JobError {
    fn code(&self) -> &'static str {
        match self {
            Self::StaleLease { .. } => "job.stale_lease",
            Self::InvalidTransition { .. } => "job.invalid_transition",
            Self::InvalidLeaseDuration => "job.invalid_lease_duration",
        }
    }
}

impl Job {
    /// A newly submitted, queued job.
    #[must_use]
    pub fn new(id: JobId, spec: JobSpec, now: Timestamp) -> Self {
        let mut job = Self::initial(id, spec.clone(), now);
        job.events.push(JobEvent::Submitted { id, spec, at: now });
        job
    }

    /// Leases the queued job to `worker` until `now + duration`, with a token greater than any
    /// before. A job is leased at most once: one whose lease expires fails instead.
    pub fn lease(
        &mut self,
        worker: WorkerId,
        duration: SignedDuration,
        now: Timestamp,
    ) -> Result<Lease, JobError> {
        if !duration.is_positive() {
            return Err(JobError::InvalidLeaseDuration);
        }
        if self.phase != JobPhase::Queued {
            return Err(self.invalid("lease"));
        }
        let token = self
            .lease
            .map_or(FencingToken::FIRST, |lease| lease.token().next());
        let lease = Lease::new(worker, token, now.saturating_add(duration));
        self.record(JobEvent::Leased { lease });
        Ok(lease)
    }

    /// Extends the live lease named by `token` until `now + duration`.
    pub fn renew(
        &mut self,
        token: FencingToken,
        duration: SignedDuration,
        now: Timestamp,
    ) -> Result<(), JobError> {
        if !duration.is_positive() {
            return Err(JobError::InvalidLeaseDuration);
        }
        self.check_token(token)?;
        let live = self.lease.is_some_and(|lease| lease.is_live(now));
        if !live || !matches!(self.phase, JobPhase::Leased | JobPhase::Running) {
            return Err(JobError::StaleLease { presented: token });
        }
        self.record(JobEvent::LeaseRenewed {
            expires_at: now.saturating_add(duration),
        });
        Ok(())
    }

    /// The holder of `token` started the job. Idempotent.
    pub fn start(&mut self, token: FencingToken) -> Result<(), JobError> {
        self.check_token(token)?;
        match self.phase {
            JobPhase::Leased => {
                self.record(JobEvent::Started);
                Ok(())
            }
            JobPhase::Running => Ok(()),
            _ => Err(self.invalid("start")),
        }
    }

    /// The holder of `token` reports the exit code. Repeating the same report is a no-op.
    pub fn finish(&mut self, token: FencingToken, exit_code: i32) -> Result<(), JobError> {
        if self.needs_completion(token, JobPhase::Finished { exit_code })? {
            self.record(JobEvent::Finished { exit_code });
        }
        Ok(())
    }

    /// The holder of `token` reports a failure. Repeating the same report is a no-op.
    pub fn fail(&mut self, token: FencingToken, reason: JobFailure) -> Result<(), JobError> {
        if self.needs_completion(token, JobPhase::Failed { reason })? {
            self.record(JobEvent::Failed { reason });
        }
        Ok(())
    }

    /// Fails the leased or running job with [`JobFailure::LeaseLost`] once its lease has expired
    /// at `now`. Does nothing while the lease is live, since a renewal may have won the race,
    /// or once the job has ended.
    pub fn lose_lease(&mut self, now: Timestamp) {
        let expired = self.lease.is_some_and(|lease| !lease.is_live(now));
        if expired && matches!(self.phase, JobPhase::Leased | JobPhase::Running) {
            self.record(JobEvent::Failed {
                reason: JobFailure::LeaseLost,
            });
        }
    }

    /// Cancels the job unless it already ended.
    pub fn cancel(&mut self) {
        if !self.phase.is_terminal() {
            self.record(JobEvent::Cancelled);
        }
    }

    /// What the job does.
    #[must_use]
    pub const fn spec(&self) -> &JobSpec {
        &self.spec
    }

    /// The current phase.
    #[must_use]
    pub const fn phase(&self) -> JobPhase {
        self.phase
    }

    /// The current or last lease.
    #[must_use]
    pub const fn current_lease(&self) -> Option<&Lease> {
        self.lease.as_ref()
    }

    /// When the job was submitted.
    #[must_use]
    pub const fn submitted_at(&self) -> Timestamp {
        self.submitted_at
    }

    fn initial(id: JobId, spec: JobSpec, at: Timestamp) -> Self {
        Self {
            id,
            spec,
            phase: JobPhase::Queued,
            lease: None,
            submitted_at: at,
            events: Vec::new(),
        }
    }

    fn record(&mut self, event: JobEvent) {
        self.apply(&event);
        self.events.push(event);
    }

    fn invalid(&self, change: &'static str) -> JobError {
        JobError::InvalidTransition {
            from: self.phase.name(),
            change,
        }
    }

    /// Ok when `token` names the current lease.
    fn check_token(&self, token: FencingToken) -> Result<(), JobError> {
        match self.lease {
            Some(lease) if lease.token() == token => Ok(()),
            _ => Err(JobError::StaleLease { presented: token }),
        }
    }

    /// Whether reaching `target` needs an event: `false` if already there, an error if the
    /// token is stale or the job cannot complete from its phase.
    fn needs_completion(&self, token: FencingToken, target: JobPhase) -> Result<bool, JobError> {
        self.check_token(token)?;
        if self.phase == target {
            return Ok(false);
        }
        if matches!(self.phase, JobPhase::Leased | JobPhase::Running) {
            Ok(true)
        } else {
            Err(self.invalid("complete"))
        }
    }
}

impl JobPhase {
    /// Whether the job can never change phase again.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Finished { .. } | Self::Failed { .. } | Self::Cancelled
        )
    }

    /// A stable lowercase name for messages.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Leased => "leased",
            Self::Running => "running",
            Self::Finished { .. } => "finished",
            Self::Failed { .. } => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl Prefixed for Job {
    const PREFIX: &'static str = "job";
}

impl Event for JobEvent {
    const SCHEMA_VERSION: u16 = 1;

    fn kind(&self) -> &'static str {
        match self {
            Self::Submitted { .. } => "igloo.job.submitted",
            Self::Leased { .. } => "igloo.job.leased",
            Self::LeaseRenewed { .. } => "igloo.job.lease_renewed",
            Self::Started => "igloo.job.started",
            Self::Finished { .. } => "igloo.job.finished",
            Self::Failed { .. } => "igloo.job.failed",
            Self::Cancelled => "igloo.job.cancelled",
        }
    }
}

impl Entity for Job {
    const NAME: &'static str = "job";

    type Event = JobEvent;

    fn id(&self) -> JobId {
        self.id
    }

    fn from_created(event: &JobEvent) -> Option<Self> {
        let JobEvent::Submitted { id, spec, at } = event else {
            return None;
        };
        Some(Self::initial(*id, spec.clone(), *at))
    }

    fn apply(&mut self, event: &JobEvent) {
        match event {
            JobEvent::Submitted { .. } => {}
            JobEvent::Leased { lease } => {
                self.lease = Some(*lease);
                self.phase = JobPhase::Leased;
            }
            JobEvent::LeaseRenewed { expires_at } => {
                self.lease = self.lease.map(|lease| lease.renewed(*expires_at));
            }
            JobEvent::Started => self.phase = JobPhase::Running,
            JobEvent::Finished { exit_code } => {
                self.phase = JobPhase::Finished {
                    exit_code: *exit_code,
                };
            }
            JobEvent::Failed { reason } => self.phase = JobPhase::Failed { reason: *reason },
            JobEvent::Cancelled => self.phase = JobPhase::Cancelled,
        }
    }

    fn take_events(&mut self) -> Vec<JobEvent> {
        std::mem::take(&mut self.events)
    }
}

/// Jobs carry no labels.
static NO_LABELS: Labels = Labels::EMPTY;

impl Resource for Job {
    type Spec = JobSpec;
    type Status = JobPhase;
    type Action = JobAction;

    fn spec(&self) -> &JobSpec {
        &self.spec
    }

    fn status(&self) -> &JobPhase {
        &self.phase
    }

    fn labels(&self) -> &Labels {
        &NO_LABELS
    }

    /// A job's spec never changes.
    fn generation(&self) -> Generation {
        Generation::INITIAL
    }

    /// A leased or running job is watched until its lease expires, then failed.
    fn plan(&self, now: Timestamp) -> Plan<JobAction> {
        let Some(lease) = self.lease else {
            return Plan::Converged;
        };
        if !matches!(self.phase, JobPhase::Leased | JobPhase::Running) {
            return Plan::Converged;
        }
        if lease.is_live(now) {
            Plan::Recheck {
                after: lease.expires_at().duration_since(now),
            }
        } else {
            Plan::Act(vec![JobAction::FailLeaseLost])
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use uuid::Uuid;

    use super::*;
    use crate::process::{Argv, EnvVars};
    use crate::testing::Scenario;

    type S = Scenario<Job>;

    const LEASE: SignedDuration = SignedDuration::from_secs(30);

    fn worker(n: u128) -> WorkerId {
        Id::from_uuid(Uuid::from_u128(n))
    }

    fn token(n: u64) -> FencingToken {
        FencingToken::from(n)
    }

    fn spec() -> JobSpec {
        JobSpec::Execute {
            sandbox: Id::from_uuid(Uuid::from_u128(9)),
            argv: Argv::try_from(vec!["cargo".to_owned(), "test".to_owned()]).expect("valid"),
            env: EnvVars::default(),
            secrets: std::collections::BTreeSet::new(),
            timeout: JobTimeout::default(),
        }
    }

    fn submitted() -> JobEvent {
        JobEvent::Submitted {
            id: S::ID,
            spec: spec(),
            at: S::NOW,
        }
    }

    fn leased(worker_n: u128, token_n: u64, expires_at: Timestamp) -> JobEvent {
        JobEvent::Leased {
            lease: Lease::new(worker(worker_n), token(token_n), expires_at),
        }
    }

    fn first_lease() -> JobEvent {
        leased(1, 1, S::NOW.saturating_add(LEASE))
    }

    #[test]
    fn submission_queues_the_job() {
        let scenario = S::create(|now| Job::new(S::ID, spec(), now)).then([submitted()]);
        assert_eq!(scenario.state().phase(), JobPhase::Queued);
    }

    #[test]
    fn the_first_lease_has_the_first_token() {
        S::given([submitted()])
            .try_when(|job, now| job.lease(worker(1), LEASE, now))
            .then([first_lease()]);
    }

    #[test]
    fn a_live_lease_cannot_be_taken() {
        S::given([submitted(), first_lease()])
            .try_when(|job, now| job.lease(worker(2), LEASE, now))
            .then_error("job.invalid_transition");
    }

    #[test]
    fn an_expired_lease_is_not_replaced() {
        S::given([submitted(), first_lease(), JobEvent::Started])
            .after(LEASE)
            .try_when(|job, now| job.lease(worker(2), LEASE, now))
            .then_error("job.invalid_transition");
    }

    #[test]
    fn a_leased_job_is_rechecked_when_its_lease_expires() {
        S::given([submitted(), first_lease()])
            .after(SignedDuration::from_secs(10))
            .plan()
            .then_recheck(SignedDuration::from_secs(20));
    }

    #[test]
    fn a_running_job_whose_lease_expired_plans_to_fail() {
        S::given([submitted(), first_lease(), JobEvent::Started])
            .after(LEASE)
            .plan()
            .then_actions([JobAction::FailLeaseLost]);
    }

    #[test]
    fn queued_and_ended_jobs_are_converged() {
        S::given([submitted()]).plan().then_converged();
        S::given([
            submitted(),
            first_lease(),
            JobEvent::Finished { exit_code: 0 },
        ])
        .after(LEASE)
        .plan()
        .then_converged();
    }

    #[test]
    fn losing_an_expired_lease_fails_the_job_and_refuses_its_late_result() {
        S::given([submitted(), first_lease(), JobEvent::Started])
            .after(LEASE)
            .when(Job::lose_lease)
            .then([JobEvent::Failed {
                reason: JobFailure::LeaseLost,
            }])
            .try_when(|job, _| job.finish(token(1), 0))
            .then_error("job.invalid_transition");
    }

    #[test]
    fn a_live_lease_is_not_lost() {
        S::given([submitted(), first_lease()])
            .when(Job::lose_lease)
            .then_no_events();
    }

    #[test]
    fn lease_durations_must_be_positive() {
        S::given([submitted()])
            .try_when(|job, now| job.lease(worker(1), SignedDuration::ZERO, now))
            .then_error("job.invalid_lease_duration");
    }

    #[test]
    fn the_holder_renews_its_live_lease() {
        S::given([submitted(), first_lease()])
            .after(SignedDuration::from_secs(10))
            .try_when(|job, now| job.renew(token(1), LEASE, now))
            .then([JobEvent::LeaseRenewed {
                expires_at: S::NOW.saturating_add(SignedDuration::from_secs(40)),
            }]);
    }

    #[test]
    fn an_expired_lease_cannot_be_renewed() {
        S::given([submitted(), first_lease()])
            .after(LEASE)
            .try_when(|job, now| job.renew(token(1), LEASE, now))
            .then_error("job.stale_lease");
    }

    #[test]
    fn starting_is_idempotent() {
        S::given([submitted(), first_lease()])
            .try_when(|job, _| job.start(token(1)))
            .then([JobEvent::Started])
            .try_when(|job, _| job.start(token(1)))
            .then_no_events();
    }

    #[test]
    fn a_stale_token_cannot_start_the_job() {
        S::given([submitted(), first_lease()])
            .try_when(|job, _| job.start(token(7)))
            .then_error("job.stale_lease");
    }

    #[test]
    fn finishing_records_the_exit_code_once() {
        S::given([submitted(), first_lease(), JobEvent::Started])
            .try_when(|job, _| job.finish(token(1), 101))
            .then([JobEvent::Finished { exit_code: 101 }])
            .try_when(|job, _| job.finish(token(1), 101))
            .then_no_events();
    }

    #[test]
    fn a_result_from_a_replaced_lease_is_rejected() {
        let second = leased(2, 2, S::NOW.saturating_add(LEASE + LEASE));
        S::given([submitted(), first_lease(), JobEvent::Started, second])
            .try_when(|job, _| job.finish(token(1), 0))
            .then_error("job.stale_lease");
    }

    #[test]
    fn a_finished_job_cannot_fail() {
        S::given([
            submitted(),
            first_lease(),
            JobEvent::Started,
            JobEvent::Finished { exit_code: 0 },
        ])
        .try_when(|job, _| job.fail(token(1), JobFailure::TimedOut))
        .then_error("job.invalid_transition");
    }

    #[test]
    fn failing_records_the_reason() {
        S::given([submitted(), first_lease()])
            .try_when(|job, _| job.fail(token(1), JobFailure::SandboxUnavailable))
            .then([JobEvent::Failed {
                reason: JobFailure::SandboxUnavailable,
            }]);
    }

    #[test]
    fn cancelling_is_idempotent_and_final() {
        S::given([submitted()])
            .when(|job, _| job.cancel())
            .then([JobEvent::Cancelled])
            .when(|job, _| job.cancel())
            .then_no_events()
            .try_when(|job, now| job.lease(worker(1), LEASE, now))
            .then_error("job.invalid_transition");
    }

    #[test]
    fn events_serialize_stably() {
        let events = vec![
            submitted(),
            first_lease(),
            JobEvent::LeaseRenewed {
                expires_at: S::NOW.saturating_add(LEASE + LEASE),
            },
            JobEvent::Started,
            JobEvent::Finished { exit_code: 0 },
            JobEvent::Failed {
                reason: JobFailure::TimedOut,
            },
            JobEvent::Cancelled,
        ];
        insta::assert_json_snapshot!(events);
    }

    #[derive(Clone, Debug)]
    enum Step {
        Lease(u128, i64),
        Wait(i64),
        Start(u64),
        Finish(u64),
    }

    fn any_step() -> impl Strategy<Value = Step> {
        prop_oneof![
            (1u128..4, 1i64..60).prop_map(|(w, secs)| Step::Lease(w, secs)),
            (1i64..90).prop_map(Step::Wait),
            (1u64..6).prop_map(Step::Start),
            (1u64..6).prop_map(Step::Finish),
        ]
    }

    proptest! {
        #[test]
        fn fencing_tokens_only_increase(steps in proptest::collection::vec(any_step(), 1..40)) {
            let mut job = Job::replay([submitted()]).expect("creation event");
            let mut now = S::NOW;
            let mut last: Option<FencingToken> = None;
            for step in steps {
                match step {
                    Step::Wait(secs) => now = now.saturating_add(SignedDuration::from_secs(secs)),
                    Step::Lease(w, secs) => {
                        if let Ok(lease) = job.lease(worker(w), SignedDuration::from_secs(secs), now) {
                            prop_assert!(last.is_none_or(|last| lease.token() > last));
                            last = Some(lease.token());
                        }
                    }
                    Step::Start(t) => { let _ = job.start(token(t)); }
                    Step::Finish(t) => { let _ = job.finish(token(t), 0); }
                }
            }
        }
    }
}
