//! Sandboxes: isolated environments started from a snapshot and converged on a worker.

mod spec;
mod status;

use serde::{Deserialize, Serialize};

use crate::worker::{Requirements, RuntimeKind, WorkerId};
use crate::{
    Entity, ErrorCode, Event, Generation, Id, Labels, Plan, Prefixed, Resource, Timestamp,
};

pub use spec::{DesiredState, Isolation, NetworkPolicy, ResourceLimits, SandboxSpec};
pub use status::{FailureReason, SandboxPhase, SandboxStatus};

/// Identifies a sandbox (`sbx_...`).
pub type SandboxId = Id<Sandbox>;

/// A sandbox: desired spec, observed status, and the generation linking the two.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sandbox {
    id: SandboxId,
    spec: SandboxSpec,
    status: SandboxStatus,
    generation: Generation,
    created_at: Timestamp,
    events: Vec<SandboxEvent>,
}

/// Facts about a sandbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SandboxEvent {
    /// The sandbox was created.
    Created {
        /// Its id.
        id: SandboxId,
        /// Its initial spec.
        spec: SandboxSpec,
        /// When.
        at: Timestamp,
    },
    /// The sandbox was assigned to a worker.
    Scheduled {
        /// The worker.
        worker: WorkerId,
    },
    /// The sandbox should stop.
    StopRequested {
        /// The new spec generation.
        generation: Generation,
    },
    /// A phase was observed.
    StatusRecorded {
        /// The phase.
        phase: SandboxPhase,
        /// The spec generation it reflects.
        observed_generation: Generation,
    },
}

/// What the sandbox controller may be asked to do. Everything else a sandbox needs happens on
/// its worker, which derives its work from the sandboxes assigned to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxAction {
    /// Choose a worker and call [`Sandbox::schedule`].
    Place,
}

/// Why a sandbox change is rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SandboxError {
    /// The change does not apply in the current phase.
    #[error("cannot {change} a sandbox that is {from}")]
    InvalidTransition {
        /// The current phase.
        from: &'static str,
        /// The rejected change.
        change: &'static str,
    },
    /// A status report names a generation newer than the spec.
    #[error("status for generation {reported} but the spec is at {current}")]
    StaleGeneration {
        /// The reported generation.
        reported: Generation,
        /// The spec's generation.
        current: Generation,
    },
    /// The spec cannot be accepted.
    #[error("invalid sandbox spec: {0}")]
    InvalidSpec(&'static str),
}

impl ErrorCode for SandboxError {
    fn code(&self) -> &'static str {
        match self {
            Self::InvalidTransition { .. } => "sandbox.invalid_transition",
            Self::StaleGeneration { .. } => "sandbox.stale_generation",
            Self::InvalidSpec(_) => "sandbox.invalid_spec",
        }
    }
}

impl Sandbox {
    /// A new pending sandbox. The spec must be desired `Running`.
    pub fn new(id: SandboxId, spec: SandboxSpec, now: Timestamp) -> Result<Self, SandboxError> {
        if spec.desired() != DesiredState::Running {
            return Err(SandboxError::InvalidSpec(
                "a new sandbox must be desired running",
            ));
        }
        let mut sandbox = Self::initial(id, spec.clone(), now);
        sandbox
            .events
            .push(SandboxEvent::Created { id, spec, at: now });
        Ok(sandbox)
    }

    /// Assigns the pending sandbox to `worker`. A no-op if already scheduled there.
    pub fn schedule(&mut self, worker: WorkerId) -> Result<(), SandboxError> {
        match self.status.phase() {
            SandboxPhase::Pending => {
                self.record(SandboxEvent::Scheduled { worker });
                Ok(())
            }
            SandboxPhase::Scheduled if self.status.worker() == Some(worker) => Ok(()),
            _ => Err(self.invalid("schedule")),
        }
    }

    /// Requests the sandbox to stop, bumping the generation. Final and idempotent. A pending
    /// sandbox, held by no worker, stops immediately.
    pub fn stop(&mut self) {
        if self.spec.desired() == DesiredState::Stopped {
            return;
        }
        let generation = self.generation.next();
        self.record(SandboxEvent::StopRequested { generation });
        if self.status.phase() == SandboxPhase::Pending {
            self.record(SandboxEvent::StatusRecorded {
                phase: SandboxPhase::Stopped,
                observed_generation: generation,
            });
        }
    }

    /// Records the phase a worker observed while acting on `observed_generation`. Repeated
    /// reports are no-ops; reports for future generations, scheduling phases or leaving a
    /// terminal phase are rejected.
    pub fn record_status(
        &mut self,
        phase: SandboxPhase,
        observed_generation: Generation,
    ) -> Result<(), SandboxError> {
        if observed_generation > self.generation {
            return Err(SandboxError::StaleGeneration {
                reported: observed_generation,
                current: self.generation,
            });
        }
        if matches!(phase, SandboxPhase::Pending | SandboxPhase::Scheduled) {
            return Err(self.invalid("record a scheduling phase for"));
        }
        let current = self.status.phase();
        let repeated =
            current == phase && self.status.observed_generation() == Some(observed_generation);
        if repeated || (current.is_terminal() && current == phase) {
            return Ok(());
        }
        if current.is_terminal() {
            return Err(self.invalid("change the phase of"));
        }
        self.record(SandboxEvent::StatusRecorded {
            phase,
            observed_generation,
        });
        Ok(())
    }

    /// When the sandbox was created.
    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    /// What a worker must offer to host this sandbox.
    #[must_use]
    pub fn requirements(&self) -> Requirements {
        match self.spec.isolation() {
            Isolation::Any => Requirements::default(),
            Isolation::Container => Requirements::builder().runtime(RuntimeKind::Oci).build(),
        }
    }

    fn initial(id: SandboxId, spec: SandboxSpec, at: Timestamp) -> Self {
        Self {
            id,
            spec,
            status: SandboxStatus::pending(),
            generation: Generation::INITIAL,
            created_at: at,
            events: Vec::new(),
        }
    }

    fn record(&mut self, event: SandboxEvent) {
        self.apply(&event);
        self.events.push(event);
    }

    fn invalid(&self, change: &'static str) -> SandboxError {
        SandboxError::InvalidTransition {
            from: self.status.phase().name(),
            change,
        }
    }
}

impl Prefixed for Sandbox {
    const PREFIX: &'static str = "sbx";
}

impl Event for SandboxEvent {
    const SCHEMA_VERSION: u16 = 1;

    fn kind(&self) -> &'static str {
        match self {
            Self::Created { .. } => "igloo.sandbox.created",
            Self::Scheduled { .. } => "igloo.sandbox.scheduled",
            Self::StopRequested { .. } => "igloo.sandbox.stop_requested",
            Self::StatusRecorded { .. } => "igloo.sandbox.status_recorded",
        }
    }
}

impl Entity for Sandbox {
    const NAME: &'static str = "sandbox";

    type Event = SandboxEvent;

    fn id(&self) -> SandboxId {
        self.id
    }

    fn from_created(event: &SandboxEvent) -> Option<Self> {
        let SandboxEvent::Created { id, spec, at } = event else {
            return None;
        };
        Some(Self::initial(*id, spec.clone(), *at))
    }

    fn apply(&mut self, event: &SandboxEvent) {
        match event {
            SandboxEvent::Created { .. } => {}
            SandboxEvent::Scheduled { worker } => self.status = self.status.scheduled(*worker),
            SandboxEvent::StopRequested { generation } => {
                self.spec.stop();
                self.generation = *generation;
            }
            SandboxEvent::StatusRecorded {
                phase,
                observed_generation,
            } => self.status = self.status.recorded(*phase, *observed_generation),
        }
    }

    fn take_events(&mut self) -> Vec<SandboxEvent> {
        std::mem::take(&mut self.events)
    }
}

impl Resource for Sandbox {
    type Spec = SandboxSpec;
    type Status = SandboxStatus;
    type Action = SandboxAction;

    fn spec(&self) -> &SandboxSpec {
        &self.spec
    }

    fn status(&self) -> &SandboxStatus {
        &self.status
    }

    fn labels(&self) -> &Labels {
        self.spec.labels()
    }

    fn generation(&self) -> Generation {
        self.generation
    }

    /// A pending sandbox needs a worker. Once placed, the worker converges it and reports back,
    /// so the server has nothing to do until those reports arrive.
    fn plan(&self, _now: Timestamp) -> Plan<SandboxAction> {
        match (self.spec.desired(), self.status.phase()) {
            (DesiredState::Running, SandboxPhase::Pending) => Plan::Act(vec![SandboxAction::Place]),
            _ => Plan::Converged,
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use uuid::Uuid;

    use super::*;
    use crate::Digest;
    use crate::snapshot::SnapshotId;
    use crate::testing::Scenario;

    type S = Scenario<Sandbox>;

    fn spec() -> SandboxSpec {
        SandboxSpec::builder()
            .snapshot(SnapshotId::from(Digest::from_blake3([7; 32])))
            .build()
    }

    fn worker() -> WorkerId {
        Id::from_uuid(Uuid::from_u128(42))
    }

    fn generation(n: u64) -> Generation {
        Generation::try_from(n).expect("non-zero")
    }

    fn created() -> SandboxEvent {
        SandboxEvent::Created {
            id: S::ID,
            spec: spec(),
            at: S::NOW,
        }
    }

    fn scheduled() -> SandboxEvent {
        SandboxEvent::Scheduled { worker: worker() }
    }

    fn recorded(phase: SandboxPhase, observed: u64) -> SandboxEvent {
        SandboxEvent::StatusRecorded {
            phase,
            observed_generation: generation(observed),
        }
    }

    fn stop_requested() -> SandboxEvent {
        SandboxEvent::StopRequested {
            generation: generation(2),
        }
    }

    #[test]
    fn creation_records_the_spec() {
        S::try_create(|now| Sandbox::new(S::ID, spec(), now)).then([created()]);
    }

    #[test]
    fn a_container_sandbox_requires_the_oci_runtime() {
        let process = crate::worker::Capabilities::new(
            crate::worker::Os::Linux,
            crate::worker::Arch::X86_64,
            [RuntimeKind::Process].into(),
            crate::worker::ProtocolVersion::V1,
        )
        .expect("one runtime");
        let any = S::given([created()]);
        assert!(process.satisfies(&any.state().requirements()).is_ok());
        let isolated = SandboxSpec::builder()
            .snapshot(SnapshotId::from(Digest::from_blake3([7; 32])))
            .isolation(Isolation::Container)
            .build();
        let isolated = S::given([SandboxEvent::Created {
            id: S::ID,
            spec: isolated,
            at: S::NOW,
        }]);
        assert!(process.satisfies(&isolated.state().requirements()).is_err());
    }

    #[test]
    fn creation_rejects_a_stopped_spec() {
        let stopped = SandboxSpec::builder()
            .snapshot(SnapshotId::from(Digest::from_blake3([7; 32])))
            .desired(DesiredState::Stopped)
            .build();
        S::try_create(|now| Sandbox::new(S::ID, stopped, now)).then_error("sandbox.invalid_spec");
    }

    #[test]
    fn scheduling_assigns_a_pending_sandbox() {
        let scenario = S::given([created()])
            .try_when(|sandbox, _| sandbox.schedule(worker()))
            .then([scheduled()]);
        assert_eq!(scenario.state().status().worker(), Some(worker()));
    }

    #[test]
    fn scheduling_again_on_the_same_worker_is_a_no_op() {
        S::given([created(), scheduled()])
            .try_when(|sandbox, _| sandbox.schedule(worker()))
            .then_no_events();
    }

    #[test]
    fn scheduling_a_running_sandbox_is_rejected() {
        S::given([created(), scheduled(), recorded(SandboxPhase::Running, 1)])
            .try_when(|sandbox, _| sandbox.schedule(worker()))
            .then_error("sandbox.invalid_transition");
    }

    #[test]
    fn stopping_bumps_the_generation() {
        let scenario = S::given([created(), scheduled()])
            .when(|sandbox, _| sandbox.stop())
            .then([stop_requested()]);
        assert_eq!(scenario.state().generation(), generation(2));
        assert_eq!(scenario.state().spec().desired(), DesiredState::Stopped);
    }

    #[test]
    fn stopping_twice_is_a_no_op() {
        S::given([created(), scheduled(), stop_requested()])
            .when(|sandbox, _| sandbox.stop())
            .then_no_events();
    }

    #[test]
    fn stopping_a_pending_sandbox_stops_it_immediately() {
        S::given([created()])
            .when(|sandbox, _| sandbox.stop())
            .then([stop_requested(), recorded(SandboxPhase::Stopped, 2)])
            .plan()
            .then_converged();
    }

    #[test]
    fn status_reports_move_the_phase() {
        S::given([created(), scheduled()])
            .try_when(|sandbox, _| sandbox.record_status(SandboxPhase::Running, generation(1)))
            .then([recorded(SandboxPhase::Running, 1)]);
    }

    #[test]
    fn repeated_status_reports_are_no_ops() {
        S::given([created(), scheduled(), recorded(SandboxPhase::Running, 1)])
            .try_when(|sandbox, _| sandbox.record_status(SandboxPhase::Running, generation(1)))
            .then_no_events();
    }

    #[test]
    fn reports_for_a_future_generation_are_rejected() {
        S::given([created(), scheduled()])
            .try_when(|sandbox, _| sandbox.record_status(SandboxPhase::Running, generation(2)))
            .then_error("sandbox.stale_generation");
    }

    #[test]
    fn a_terminal_phase_is_never_left() {
        S::given([created(), scheduled(), recorded(SandboxPhase::Stopped, 1)])
            .try_when(|sandbox, _| sandbox.record_status(SandboxPhase::Running, generation(1)))
            .then_error("sandbox.invalid_transition");
    }

    #[test]
    fn workers_cannot_report_scheduling_phases() {
        S::given([created(), scheduled()])
            .try_when(|sandbox, _| sandbox.record_status(SandboxPhase::Pending, generation(1)))
            .then_error("sandbox.invalid_transition");
    }

    #[test]
    fn a_pending_sandbox_is_placed() {
        S::given([created()])
            .plan()
            .then_actions([SandboxAction::Place]);
    }

    #[test]
    fn a_running_sandbox_is_converged() {
        S::given([created(), scheduled(), recorded(SandboxPhase::Running, 1)])
            .plan()
            .then_converged();
    }

    #[test]
    fn a_sandbox_desired_stopped_on_a_worker_waits_for_the_worker() {
        S::given([created(), scheduled(), stop_requested()])
            .plan()
            .then_converged();
    }

    #[test]
    fn a_failed_sandbox_is_converged() {
        let failed = SandboxPhase::Failed {
            reason: FailureReason::WorkerLost,
        };
        S::given([created(), scheduled(), recorded(failed, 1)])
            .plan()
            .then_converged();
    }

    #[test]
    fn a_scheduled_sandbox_waits_for_its_worker() {
        S::given([created(), scheduled()]).plan().then_converged();
    }

    #[test]
    fn events_serialize_stably() {
        let events = vec![
            created(),
            scheduled(),
            stop_requested(),
            recorded(
                SandboxPhase::Failed {
                    reason: FailureReason::RuntimeError,
                },
                2,
            ),
        ];
        insta::assert_json_snapshot!(events);
    }

    fn any_phase() -> impl Strategy<Value = SandboxPhase> {
        prop_oneof![
            Just(SandboxPhase::Starting),
            Just(SandboxPhase::Running),
            Just(SandboxPhase::Stopping),
            Just(SandboxPhase::Stopped),
            Just(SandboxPhase::Failed {
                reason: FailureReason::RuntimeError
            }),
        ]
    }

    proptest! {
        #[test]
        fn terminal_phases_are_final(phases in proptest::collection::vec(any_phase(), 1..20)) {
            let mut sandbox = Sandbox::replay([created(), scheduled()]).expect("creation event");
            let mut terminal: Option<SandboxPhase> = None;
            for phase in phases {
                let _ = sandbox.record_status(phase, generation(1));
                let current = sandbox.status().phase();
                if let Some(terminal) = terminal {
                    prop_assert_eq!(current, terminal);
                }
                if current.is_terminal() {
                    terminal = Some(current);
                }
            }
        }
    }
}
