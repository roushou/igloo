//! Workers: machines that run sandboxes, tracked through their connection to the server.

mod capabilities;
mod usage;

use jiff::SignedDuration;
use serde::{Deserialize, Serialize};

use crate::{
    Entity, ErrorCode, Event, Generation, Id, Labels, Plan, Prefixed, Resource, Timestamp,
};

pub use capabilities::{
    Arch, Capabilities, Mismatch, NoRuntime, Os, ProtocolVersion, Requirements, RuntimeKind,
};
pub use usage::Usage;

/// Identifies a worker (`wrk_...`).
pub type WorkerId = Id<Worker>;

/// A worker as the server sees it.
///
/// Invariant: a `Lost` worker never reconnects under the same id; it registers again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worker {
    id: WorkerId,
    capabilities: Capabilities,
    labels: Labels,
    schedulability: Schedulability,
    connection: Connection,
    generation: Generation,
    events: Vec<WorkerEvent>,
}

/// Whether new work may be placed on a worker. The worker's spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Schedulability {
    /// Accepts new work.
    Schedulable,
    /// Finishes current work, accepts nothing new.
    Draining,
}

/// The worker's connection to the server. The worker's status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Connection {
    /// The worker's stream is open.
    Connected,
    /// The stream closed at `since`; the worker may still come back.
    Disconnected {
        /// When the stream closed.
        since: Timestamp,
    },
    /// The worker stayed away too long; its work is considered gone.
    Lost,
}

/// Facts about a worker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerEvent {
    /// The worker registered and is connected.
    Registered {
        /// Its id.
        id: WorkerId,
        /// What it offers.
        capabilities: Capabilities,
        /// Its labels.
        labels: Labels,
    },
    /// The worker reconnected.
    Connected {
        /// What it offers now.
        capabilities: Capabilities,
    },
    /// The worker's stream closed.
    Disconnected {
        /// When.
        at: Timestamp,
    },
    /// New work is no longer placed on the worker.
    DrainRequested {
        /// The new spec generation.
        generation: Generation,
    },
    /// The worker is considered gone.
    MarkedLost,
}

/// What the worker controller may be asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerAction {
    /// Call [`Worker::mark_lost`].
    MarkLost,
}

/// Why a worker change is rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkerError {
    /// The change does not apply to the worker's connection state.
    #[error("cannot {change} a worker that is {from}")]
    InvalidTransition {
        /// The connection state.
        from: &'static str,
        /// The rejected change.
        change: &'static str,
    },
}

impl ErrorCode for WorkerError {
    fn code(&self) -> &'static str {
        match self {
            Self::InvalidTransition { .. } => "worker.invalid_transition",
        }
    }
}

impl Worker {
    /// How long a disconnected worker may stay away before it is lost.
    pub const RECONNECT_GRACE: SignedDuration = SignedDuration::from_secs(30);

    /// A newly registered, connected, schedulable worker.
    #[must_use]
    pub fn new(id: WorkerId, capabilities: Capabilities, labels: Labels) -> Self {
        let mut worker = Self::initial(id, capabilities.clone(), labels.clone());
        worker.events.push(WorkerEvent::Registered {
            id,
            capabilities,
            labels,
        });
        worker
    }

    /// The worker's stream opened, advertising `capabilities`. A lost worker must register
    /// again instead.
    pub fn connect(&mut self, capabilities: Capabilities) -> Result<(), WorkerError> {
        match self.connection {
            Connection::Lost => Err(self.invalid("connect")),
            Connection::Connected if capabilities == self.capabilities => Ok(()),
            _ => {
                self.record(WorkerEvent::Connected { capabilities });
                Ok(())
            }
        }
    }

    /// The worker's stream closed at `now`. Idempotent.
    pub fn disconnect(&mut self, now: Timestamp) {
        if self.connection == Connection::Connected {
            self.record(WorkerEvent::Disconnected { at: now });
        }
    }

    /// Stops placing new work on the worker. Idempotent.
    pub fn drain(&mut self) {
        if self.schedulability == Schedulability::Schedulable {
            let generation = self.generation.next();
            self.record(WorkerEvent::DrainRequested { generation });
        }
    }

    /// Gives up on a disconnected worker. Idempotent; a connected worker cannot be lost.
    pub fn mark_lost(&mut self) -> Result<(), WorkerError> {
        match self.connection {
            Connection::Connected => Err(self.invalid("mark lost")),
            Connection::Disconnected { .. } => {
                self.record(WorkerEvent::MarkedLost);
                Ok(())
            }
            Connection::Lost => Ok(()),
        }
    }

    /// What the worker offers.
    #[must_use]
    pub const fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// Whether new work may be placed on the worker now.
    #[must_use]
    pub fn is_schedulable(&self) -> bool {
        self.connection == Connection::Connected
            && self.schedulability == Schedulability::Schedulable
    }

    fn initial(id: WorkerId, capabilities: Capabilities, labels: Labels) -> Self {
        Self {
            id,
            capabilities,
            labels,
            schedulability: Schedulability::Schedulable,
            connection: Connection::Connected,
            generation: Generation::INITIAL,
            events: Vec::new(),
        }
    }

    fn record(&mut self, event: WorkerEvent) {
        self.apply(&event);
        self.events.push(event);
    }

    fn invalid(&self, change: &'static str) -> WorkerError {
        WorkerError::InvalidTransition {
            from: self.connection.name(),
            change,
        }
    }
}

impl Connection {
    /// A stable lowercase name for messages.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Disconnected { .. } => "disconnected",
            Self::Lost => "lost",
        }
    }
}

impl Prefixed for Worker {
    const PREFIX: &'static str = "wrk";
}

impl Event for WorkerEvent {
    const SCHEMA_VERSION: u16 = 1;

    fn kind(&self) -> &'static str {
        match self {
            Self::Registered { .. } => "igloo.worker.registered",
            Self::Connected { .. } => "igloo.worker.connected",
            Self::Disconnected { .. } => "igloo.worker.disconnected",
            Self::DrainRequested { .. } => "igloo.worker.drain_requested",
            Self::MarkedLost => "igloo.worker.marked_lost",
        }
    }
}

impl Entity for Worker {
    const NAME: &'static str = "worker";

    type Event = WorkerEvent;

    fn id(&self) -> WorkerId {
        self.id
    }

    fn from_created(event: &WorkerEvent) -> Option<Self> {
        let WorkerEvent::Registered {
            id,
            capabilities,
            labels,
        } = event
        else {
            return None;
        };
        Some(Self::initial(*id, capabilities.clone(), labels.clone()))
    }

    fn apply(&mut self, event: &WorkerEvent) {
        match event {
            WorkerEvent::Registered { .. } => {}
            WorkerEvent::Connected { capabilities } => {
                self.capabilities = capabilities.clone();
                self.connection = Connection::Connected;
            }
            WorkerEvent::Disconnected { at } => {
                self.connection = Connection::Disconnected { since: *at };
            }
            WorkerEvent::DrainRequested { generation } => {
                self.schedulability = Schedulability::Draining;
                self.generation = *generation;
            }
            WorkerEvent::MarkedLost => self.connection = Connection::Lost,
        }
    }

    fn take_events(&mut self) -> Vec<WorkerEvent> {
        std::mem::take(&mut self.events)
    }
}

impl Resource for Worker {
    type Spec = Schedulability;
    type Status = Connection;
    type Action = WorkerAction;

    fn spec(&self) -> &Schedulability {
        &self.schedulability
    }

    fn status(&self) -> &Connection {
        &self.connection
    }

    fn labels(&self) -> &Labels {
        &self.labels
    }

    fn generation(&self) -> Generation {
        self.generation
    }

    fn plan(&self, now: Timestamp) -> Plan<WorkerAction> {
        match self.connection {
            Connection::Connected | Connection::Lost => Plan::Converged,
            Connection::Disconnected { since } => {
                let remaining = Self::RECONNECT_GRACE - now.duration_since(since);
                if remaining.is_positive() {
                    Plan::Recheck { after: remaining }
                } else {
                    Plan::Act(vec![WorkerAction::MarkLost])
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::testing::Scenario;

    type S = Scenario<Worker>;

    fn capabilities(runtime: RuntimeKind) -> Capabilities {
        Capabilities::new(
            Os::Linux,
            Arch::X86_64,
            BTreeSet::from([runtime]),
            ProtocolVersion::V1,
        )
        .expect("one runtime")
    }

    fn registered() -> WorkerEvent {
        WorkerEvent::Registered {
            id: S::ID,
            capabilities: capabilities(RuntimeKind::Process),
            labels: Labels::default(),
        }
    }

    fn disconnected() -> WorkerEvent {
        WorkerEvent::Disconnected { at: S::NOW }
    }

    #[test]
    fn registration_connects_the_worker() {
        let scenario = S::create(|_| {
            Worker::new(S::ID, capabilities(RuntimeKind::Process), Labels::default())
        })
        .then([registered()]);
        assert!(scenario.state().is_schedulable());
        scenario.plan().then_converged();
    }

    #[test]
    fn reconnecting_with_the_same_capabilities_is_a_no_op() {
        S::given([registered()])
            .try_when(|worker, _| worker.connect(capabilities(RuntimeKind::Process)))
            .then_no_events();
    }

    #[test]
    fn reconnecting_updates_capabilities() {
        S::given([registered(), disconnected()])
            .try_when(|worker, _| worker.connect(capabilities(RuntimeKind::Oci)))
            .then([WorkerEvent::Connected {
                capabilities: capabilities(RuntimeKind::Oci),
            }]);
    }

    #[test]
    fn a_lost_worker_cannot_reconnect() {
        S::given([registered(), disconnected(), WorkerEvent::MarkedLost])
            .try_when(|worker, _| worker.connect(capabilities(RuntimeKind::Process)))
            .then_error("worker.invalid_transition");
    }

    #[test]
    fn disconnecting_is_idempotent() {
        S::given([registered()])
            .when(Worker::disconnect)
            .then([disconnected()])
            .when(Worker::disconnect)
            .then_no_events();
    }

    #[test]
    fn draining_bumps_the_generation_once() {
        let scenario = S::given([registered()])
            .when(|worker, _| worker.drain())
            .then([WorkerEvent::DrainRequested {
                generation: Generation::INITIAL.next(),
            }]);
        assert!(!scenario.state().is_schedulable());
        scenario.when(|worker, _| worker.drain()).then_no_events();
    }

    #[test]
    fn a_connected_worker_cannot_be_marked_lost() {
        S::given([registered()])
            .try_when(|worker, _| worker.mark_lost())
            .then_error("worker.invalid_transition");
    }

    #[test]
    fn marking_lost_is_idempotent() {
        S::given([registered(), disconnected()])
            .try_when(|worker, _| worker.mark_lost())
            .then([WorkerEvent::MarkedLost])
            .try_when(|worker, _| worker.mark_lost())
            .then_no_events()
            .plan()
            .then_converged();
    }

    #[test]
    fn a_disconnected_worker_is_rechecked_until_the_grace_period_ends() {
        let scenario = S::given([registered(), disconnected()]);
        scenario.plan().then_recheck(Worker::RECONNECT_GRACE);
        scenario
            .after(Worker::RECONNECT_GRACE)
            .plan()
            .then_actions([WorkerAction::MarkLost]);
    }

    #[test]
    fn events_serialize_stably() {
        let events = vec![
            registered(),
            WorkerEvent::Connected {
                capabilities: capabilities(RuntimeKind::Oci),
            },
            disconnected(),
            WorkerEvent::DrainRequested {
                generation: Generation::INITIAL.next(),
            },
            WorkerEvent::MarkedLost,
        ];
        insta::assert_json_snapshot!(events);
    }
}
