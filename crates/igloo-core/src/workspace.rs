//! Workspaces: a person's long-lived sandbox on a branch of a repository. It outlives its
//! sandbox: stopping seals the sandbox's changes into a snapshot, and starting resumes from it.

use std::collections::BTreeSet;

use jiff::SignedDuration;
use serde::{Deserialize, Serialize};

use crate::actor::UserId;
use crate::job::JobId;
use crate::repo::{BranchName, RepoId, SecretName};
use crate::sandbox::SandboxId;
use crate::seal::SealId;
use crate::snapshot::SnapshotId;
use crate::{
    Entity, ErrorCode, Event, Generation, Id, Labels, Plan, Prefixed, Resource, Timestamp,
};

/// Identifies a workspace (`wsp_...`).
pub type WorkspaceId = Id<Workspace>;

/// A workspace: the desired state a person declared, and where its sandbox and sealed
/// snapshot are.
///
/// Invariants: at most one sandbox is live at a time; a stop seals the sandbox before stopping
/// it, unless the sandbox ended on its own or the workspace is deleted; `snapshot` only ever
/// moves to the result of a seal; a workspace whose sandbox ended unexpectedly is desired
/// stopped, never restarted by itself; a deleted workspace never starts again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    id: WorkspaceId,
    spec: WorkspaceSpec,
    generation: Generation,
    status: WorkspaceStatus,
    progress: Progress,
    deleted: bool,
    created_at: Timestamp,
    events: Vec<WorkspaceEvent>,
}

/// What a person declared about a workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSpec {
    /// The person the workspace belongs to.
    pub owner: UserId,
    /// The repository it works on.
    pub repo: RepoId,
    /// The branch it was opened on.
    pub branch: BranchName,
    /// Whether its sandbox should run.
    pub desired: Desired,
}

/// Whether a workspace's sandbox should run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Desired {
    /// A sandbox should be running.
    Running,
    /// No sandbox should be running; the changes are sealed.
    Stopped,
}

/// What is observed of a workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceStatus {
    phase: WorkspacePhase,
    sandbox: Option<SandboxId>,
    snapshot: Option<SnapshotId>,
    secrets: BTreeSet<SecretName>,
    last_activity: Timestamp,
    setup: Option<SetupResult>,
}

/// How setting up a workspace's sandbox ended: the job that did it and whether it worked.
///
/// Invariant: a failed setup leaves the workspace usable; it only tells the person that their
/// checkout or dotfiles may not be as configured.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupResult {
    /// The job that ran the setup; its logs hold the output.
    pub job: JobId,
    /// How it ended.
    pub outcome: SetupOutcome,
}

/// Whether a workspace's setup worked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SetupOutcome {
    /// The setup job exited 0.
    Succeeded,
    /// The setup job exited non-zero or did not complete.
    Failed {
        /// Why, in a short sentence.
        reason: String,
    },
}

/// Where a workspace is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspacePhase {
    /// Opening: its snapshot is being prepared, or its sandbox is starting.
    Starting,
    /// Its sandbox runs.
    Running,
    /// Its changes are being sealed and its sandbox stopped.
    Stopping,
    /// No sandbox exists; the sealed snapshot, if any, is what the next start resumes from.
    Stopped,
}

/// The steps between the phases, one at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Progress {
    Stopped,
    /// Desired running, no sandbox yet.
    Opening,
    /// The sandbox exists and has not reported running.
    Booting {
        sandbox: SandboxId,
    },
    Running {
        sandbox: SandboxId,
    },
    /// Stopping: the seal is requested, then awaited. `ended` when the sandbox ended while the
    /// seal was in flight.
    Sealing {
        sandbox: SandboxId,
        seal: Option<SealId>,
        ended: bool,
    },
    /// Stopping: the sandbox is to be stopped, and `requested` once it has been.
    Halting {
        sandbox: SandboxId,
        requested: bool,
    },
}

/// What the workspace controller may be asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceAction {
    /// Prepare the snapshot, create the sandbox and record it.
    Open,
    /// Seal the sandbox.
    Seal(SandboxId),
    /// Stop the sandbox.
    StopSandbox(SandboxId),
    /// Stop the workspace if it is still idle.
    StopIdle,
}

/// Facts about a workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceEvent {
    /// A workspace was created, desired running.
    Created {
        /// Its id.
        id: WorkspaceId,
        /// What was declared.
        spec: WorkspaceSpec,
        /// When.
        at: Timestamp,
    },
    /// The workspace should run.
    StartRequested {
        /// The new spec generation.
        generation: Generation,
        /// When.
        at: Timestamp,
    },
    /// The workspace should stop.
    StopRequested {
        /// The new spec generation.
        generation: Generation,
    },
    /// The workspace was deleted: stopped without sealing, and never started again.
    Deleted {
        /// The new spec generation.
        generation: Generation,
        /// When.
        at: Timestamp,
    },
    /// The sandbox was created.
    SandboxCreated {
        /// The sandbox.
        sandbox: SandboxId,
        /// The repository secrets granted to terminals in it.
        secrets: BTreeSet<SecretName>,
    },
    /// The sandbox runs.
    SandboxRunning {
        /// The sandbox.
        sandbox: SandboxId,
        /// When.
        at: Timestamp,
    },
    /// The sandbox is being sealed.
    Sealing {
        /// The seal.
        seal: SealId,
    },
    /// The seal produced this snapshot.
    Sealed {
        /// The snapshot, which holds everything the workspace has.
        snapshot: SnapshotId,
    },
    /// The seal failed; the changes since the previous seal are lost.
    SealFailed,
    /// The sandbox was asked to stop.
    SandboxStopRequested,
    /// The sandbox ended.
    SandboxEnded {
        /// The sandbox.
        sandbox: SandboxId,
    },
    /// A person used the workspace.
    ActivityRecorded {
        /// When.
        at: Timestamp,
    },
    /// The job setting up the sandbox ended.
    SetupRecorded {
        /// The sandbox that was set up.
        sandbox: SandboxId,
        /// The setup job.
        job: JobId,
        /// How it ended.
        outcome: SetupOutcome,
    },
}

/// Why a workspace change is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WorkspaceError {
    /// The workspace was deleted.
    #[error("the workspace was deleted")]
    Deleted,
    /// The change does not apply to the workspace's state.
    #[error("the workspace is not at that step")]
    OutOfOrder,
}

impl ErrorCode for WorkspaceError {
    fn code(&self) -> &'static str {
        match self {
            Self::Deleted => "workspace.deleted",
            Self::OutOfOrder => "workspace.out_of_order",
        }
    }
}

impl Workspace {
    /// How long a running workspace may go without a person using it before it stops itself.
    pub const IDLE_TIMEOUT: SignedDuration = SignedDuration::from_hours(2);

    /// A workspace of `owner` on `branch` of `repo`, desired running.
    #[must_use]
    pub fn new(
        id: WorkspaceId,
        owner: UserId,
        repo: RepoId,
        branch: BranchName,
        now: Timestamp,
    ) -> Self {
        let spec = WorkspaceSpec {
            owner,
            repo,
            branch,
            desired: Desired::Running,
        };
        let mut workspace = Self::initial(id, spec.clone(), now);
        workspace
            .events
            .push(WorkspaceEvent::Created { id, spec, at: now });
        workspace
    }

    /// Declares the workspace should run. A no-op when it is already desired running.
    pub fn start(&mut self, now: Timestamp) -> Result<(), WorkspaceError> {
        if self.deleted {
            return Err(WorkspaceError::Deleted);
        }
        if self.spec.desired == Desired::Stopped {
            self.record(WorkspaceEvent::StartRequested {
                generation: self.generation.next(),
                at: now,
            });
        }
        Ok(())
    }

    /// Declares the workspace should stop. A no-op when it is already desired stopped.
    pub fn stop(&mut self) {
        if self.spec.desired == Desired::Running {
            self.record(WorkspaceEvent::StopRequested {
                generation: self.generation.next(),
            });
        }
    }

    /// Stops the workspace if it is running and has been idle for [`Self::IDLE_TIMEOUT`] at
    /// `now`.
    pub fn stop_if_idle(&mut self, now: Timestamp) {
        if matches!(self.progress, Progress::Running { .. }) && self.idle_for(now) {
            self.stop();
        }
    }

    /// Deletes the workspace: its sandbox is stopped without sealing and it never starts again.
    /// A no-op when already deleted.
    pub fn delete(&mut self, now: Timestamp) {
        if !self.deleted {
            self.record(WorkspaceEvent::Deleted {
                generation: self.generation.next(),
                at: now,
            });
        }
    }

    /// Records the sandbox opened for the workspace and the secrets it grants.
    pub fn sandbox_created(
        &mut self,
        sandbox: SandboxId,
        secrets: BTreeSet<SecretName>,
    ) -> Result<(), WorkspaceError> {
        if self.progress != Progress::Opening {
            return Err(WorkspaceError::OutOfOrder);
        }
        self.record(WorkspaceEvent::SandboxCreated { sandbox, secrets });
        Ok(())
    }

    /// Records that `sandbox` runs. Ignored unless it is the sandbox being started.
    pub fn sandbox_running(&mut self, sandbox: SandboxId, now: Timestamp) {
        if self.progress == (Progress::Booting { sandbox }) {
            self.record(WorkspaceEvent::SandboxRunning { sandbox, at: now });
        }
    }

    /// Records the seal of the sandbox being stopped.
    pub fn sealing(&mut self, seal: SealId) -> Result<(), WorkspaceError> {
        match self.progress {
            Progress::Sealing { seal: None, .. } => {
                self.record(WorkspaceEvent::Sealing { seal });
                Ok(())
            }
            Progress::Sealing {
                seal: Some(current),
                ..
            } if current == seal => Ok(()),
            _ => Err(WorkspaceError::OutOfOrder),
        }
    }

    /// Records how the workspace's seal ended: the snapshot, or `Err` when it failed. Other
    /// seals are ignored.
    pub fn seal_ended(&mut self, seal: SealId, snapshot: Result<SnapshotId, ()>) {
        if matches!(self.progress, Progress::Sealing { seal: Some(current), .. } if current == seal)
        {
            self.record(match snapshot {
                Ok(snapshot) => WorkspaceEvent::Sealed { snapshot },
                Err(()) => WorkspaceEvent::SealFailed,
            });
        }
    }

    /// Records that the sandbox was asked to stop.
    pub fn sandbox_stopping(&mut self) -> Result<(), WorkspaceError> {
        match self.progress {
            Progress::Halting {
                requested: false, ..
            } => {
                self.record(WorkspaceEvent::SandboxStopRequested);
                Ok(())
            }
            Progress::Halting {
                requested: true, ..
            } => Ok(()),
            _ => Err(WorkspaceError::OutOfOrder),
        }
    }

    /// Records that `sandbox` ended. A sandbox ending while the workspace should run leaves
    /// the workspace stopped. Ignored unless it is the workspace's sandbox.
    pub fn sandbox_ended(&mut self, sandbox: SandboxId) {
        if self.progress.sandbox() != Some(sandbox) {
            return;
        }
        if matches!(
            self.progress,
            Progress::Booting { .. } | Progress::Running { .. }
        ) {
            self.stop();
        }
        self.record(WorkspaceEvent::SandboxEnded { sandbox });
    }

    /// Records how the job setting up `sandbox` ended. Ignored unless `sandbox` is the
    /// workspace's sandbox, and when the same result is already recorded.
    pub fn setup_ended(&mut self, sandbox: SandboxId, job: JobId, outcome: SetupOutcome) {
        let result = SetupResult { job, outcome };
        if self.progress.sandbox() == Some(sandbox) && self.status.setup.as_ref() != Some(&result) {
            self.record(WorkspaceEvent::SetupRecorded {
                sandbox,
                job: result.job,
                outcome: result.outcome,
            });
        }
    }

    /// Records that a person used the running workspace at `now`.
    pub fn touch(&mut self, now: Timestamp) {
        if matches!(self.progress, Progress::Running { .. }) {
            self.record(WorkspaceEvent::ActivityRecorded { at: now });
        }
    }

    /// Whether it was deleted.
    #[must_use]
    pub const fn is_deleted(&self) -> bool {
        self.deleted
    }

    /// The seal in flight, while stopping.
    #[must_use]
    pub const fn seal(&self) -> Option<SealId> {
        match self.progress {
            Progress::Sealing { seal, .. } => seal,
            _ => None,
        }
    }

    /// When it was created.
    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    fn idle_for(&self, now: Timestamp) -> bool {
        now.duration_since(self.status.last_activity) >= Self::IDLE_TIMEOUT
    }

    fn initial(id: WorkspaceId, spec: WorkspaceSpec, at: Timestamp) -> Self {
        Self {
            id,
            spec,
            generation: Generation::INITIAL,
            status: WorkspaceStatus {
                phase: WorkspacePhase::Starting,
                sandbox: None,
                snapshot: None,
                secrets: BTreeSet::new(),
                last_activity: at,
                setup: None,
            },
            progress: Progress::Opening,
            deleted: false,
            created_at: at,
            events: Vec::new(),
        }
    }

    fn record(&mut self, event: WorkspaceEvent) {
        self.apply(&event);
        self.events.push(event);
    }

    /// After a sandbox is gone: opening again when a start is waiting, else stopped.
    fn settle(&mut self) {
        self.progress = if self.spec.desired == Desired::Running {
            Progress::Opening
        } else {
            Progress::Stopped
        };
    }

    /// Where a stop request leaves the current step.
    fn stopping(&mut self) {
        self.progress = match self.progress {
            Progress::Opening => Progress::Stopped,
            Progress::Booting { sandbox } => Progress::Halting {
                sandbox,
                requested: false,
            },
            Progress::Running { sandbox } => Progress::Sealing {
                sandbox,
                seal: None,
                ended: false,
            },
            other => other,
        };
    }
}

impl Progress {
    const fn sandbox(self) -> Option<SandboxId> {
        match self {
            Self::Stopped | Self::Opening => None,
            Self::Booting { sandbox }
            | Self::Running { sandbox }
            | Self::Sealing { sandbox, .. }
            | Self::Halting { sandbox, .. } => Some(sandbox),
        }
    }

    const fn phase(self) -> WorkspacePhase {
        match self {
            Self::Stopped => WorkspacePhase::Stopped,
            Self::Opening | Self::Booting { .. } => WorkspacePhase::Starting,
            Self::Running { .. } => WorkspacePhase::Running,
            Self::Sealing { .. } | Self::Halting { .. } => WorkspacePhase::Stopping,
        }
    }
}

impl WorkspaceStatus {
    /// Where the workspace is.
    #[must_use]
    pub const fn phase(&self) -> WorkspacePhase {
        self.phase
    }

    /// The sandbox it runs in, from its creation until it ends.
    #[must_use]
    pub const fn sandbox(&self) -> Option<SandboxId> {
        self.sandbox
    }

    /// The snapshot its last stop sealed: everything the workspace has, which the next start
    /// resumes from.
    #[must_use]
    pub const fn snapshot(&self) -> Option<SnapshotId> {
        self.snapshot
    }

    /// The repository secrets granted to terminals in its latest sandbox.
    #[must_use]
    pub const fn secrets(&self) -> &BTreeSet<SecretName> {
        &self.secrets
    }

    /// When a person last used it, or when it started.
    #[must_use]
    pub const fn last_activity(&self) -> Timestamp {
        self.last_activity
    }

    /// How setting up its latest unsealed sandbox ended, once it did: a failure here means the
    /// checkout or dotfiles may not be as configured.
    #[must_use]
    pub const fn setup(&self) -> Option<&SetupResult> {
        self.setup.as_ref()
    }
}

impl WorkspacePhase {
    /// A stable lowercase name for messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
        }
    }
}

impl Prefixed for Workspace {
    const PREFIX: &'static str = "wsp";
}

impl Event for WorkspaceEvent {
    const SCHEMA_VERSION: u16 = 1;

    fn kind(&self) -> &'static str {
        match self {
            Self::Created { .. } => "igloo.workspace.created",
            Self::StartRequested { .. } => "igloo.workspace.start_requested",
            Self::StopRequested { .. } => "igloo.workspace.stop_requested",
            Self::Deleted { .. } => "igloo.workspace.deleted",
            Self::SandboxCreated { .. } => "igloo.workspace.sandbox_created",
            Self::SandboxRunning { .. } => "igloo.workspace.sandbox_running",
            Self::Sealing { .. } => "igloo.workspace.sealing",
            Self::Sealed { .. } => "igloo.workspace.sealed",
            Self::SealFailed => "igloo.workspace.seal_failed",
            Self::SandboxStopRequested => "igloo.workspace.sandbox_stop_requested",
            Self::SandboxEnded { .. } => "igloo.workspace.sandbox_ended",
            Self::ActivityRecorded { .. } => "igloo.workspace.activity_recorded",
            Self::SetupRecorded { .. } => "igloo.workspace.setup_recorded",
        }
    }
}

impl Entity for Workspace {
    const NAME: &'static str = "workspace";

    type Event = WorkspaceEvent;

    fn id(&self) -> WorkspaceId {
        self.id
    }

    fn from_created(event: &WorkspaceEvent) -> Option<Self> {
        let WorkspaceEvent::Created { id, spec, at } = event else {
            return None;
        };
        Some(Self::initial(*id, spec.clone(), *at))
    }

    fn apply(&mut self, event: &WorkspaceEvent) {
        match event {
            WorkspaceEvent::Created { .. } => {}
            WorkspaceEvent::StartRequested { generation, at } => {
                self.spec.desired = Desired::Running;
                self.generation = *generation;
                if self.progress == Progress::Stopped {
                    self.progress = Progress::Opening;
                    self.status.last_activity = *at;
                }
            }
            WorkspaceEvent::StopRequested { generation } => {
                self.spec.desired = Desired::Stopped;
                self.generation = *generation;
                self.stopping();
            }
            WorkspaceEvent::Deleted { generation, .. } => {
                self.deleted = true;
                self.spec.desired = Desired::Stopped;
                self.generation = *generation;
                self.progress = match self.progress {
                    Progress::Opening => Progress::Stopped,
                    Progress::Booting { sandbox }
                    | Progress::Running { sandbox }
                    | Progress::Sealing {
                        sandbox,
                        seal: None,
                        ..
                    } => Progress::Halting {
                        sandbox,
                        requested: false,
                    },
                    other => other,
                };
            }
            WorkspaceEvent::SandboxCreated { sandbox, secrets } => {
                self.progress = Progress::Booting { sandbox: *sandbox };
                self.status.secrets.clone_from(secrets);
                if self.status.snapshot.is_none() {
                    self.status.setup = None;
                }
            }
            WorkspaceEvent::SandboxRunning { sandbox, at } => {
                self.progress = Progress::Running { sandbox: *sandbox };
                self.status.last_activity = *at;
            }
            WorkspaceEvent::Sealing { seal } => {
                if let Progress::Sealing { seal: slot, .. } = &mut self.progress {
                    *slot = Some(*seal);
                }
            }
            WorkspaceEvent::Sealed { snapshot } => {
                self.status.snapshot = Some(*snapshot);
                self.sealed();
            }
            WorkspaceEvent::SealFailed => self.sealed(),
            WorkspaceEvent::SandboxStopRequested => {
                if let Progress::Halting { requested, .. } = &mut self.progress {
                    *requested = true;
                }
            }
            WorkspaceEvent::SandboxEnded { .. } => {
                if let Progress::Sealing {
                    seal: Some(_),
                    ended,
                    ..
                } = &mut self.progress
                {
                    *ended = true;
                } else {
                    self.settle();
                }
            }
            WorkspaceEvent::ActivityRecorded { at } => {
                self.status.last_activity = self.status.last_activity.max(*at);
            }
            WorkspaceEvent::SetupRecorded { job, outcome, .. } => {
                self.status.setup = Some(SetupResult {
                    job: *job,
                    outcome: outcome.clone(),
                });
            }
        }
        self.status.phase = self.progress.phase();
        self.status.sandbox = self.progress.sandbox();
    }

    fn take_events(&mut self) -> Vec<WorkspaceEvent> {
        std::mem::take(&mut self.events)
    }
}

impl Workspace {
    /// A seal ended: stop its sandbox, which is already gone if it ended meanwhile.
    fn sealed(&mut self) {
        if let Progress::Sealing { sandbox, ended, .. } = self.progress {
            if ended {
                self.settle();
            } else {
                self.progress = Progress::Halting {
                    sandbox,
                    requested: false,
                };
            }
        }
    }
}

/// Workspaces carry no labels.
static NO_LABELS: Labels = Labels::EMPTY;

impl Resource for Workspace {
    type Spec = WorkspaceSpec;
    type Status = WorkspaceStatus;
    type Action = WorkspaceAction;

    fn spec(&self) -> &WorkspaceSpec {
        &self.spec
    }

    fn status(&self) -> &WorkspaceStatus {
        &self.status
    }

    fn labels(&self) -> &Labels {
        &NO_LABELS
    }

    fn generation(&self) -> Generation {
        self.generation
    }

    /// One step at a time; steps waiting on the sandbox or the seal resume when their outcome
    /// is recorded. A running workspace is rechecked when it would become idle.
    fn plan(&self, now: Timestamp) -> Plan<WorkspaceAction> {
        match self.progress {
            Progress::Opening => Plan::Act(vec![WorkspaceAction::Open]),
            Progress::Running { .. } if self.idle_for(now) => {
                Plan::Act(vec![WorkspaceAction::StopIdle])
            }
            Progress::Running { .. } => Plan::Recheck {
                after: self
                    .status
                    .last_activity
                    .saturating_add(Self::IDLE_TIMEOUT)
                    .duration_since(now),
            },
            Progress::Sealing {
                sandbox,
                seal: None,
                ..
            } => Plan::Act(vec![WorkspaceAction::Seal(sandbox)]),
            Progress::Halting {
                sandbox,
                requested: false,
            } => Plan::Act(vec![WorkspaceAction::StopSandbox(sandbox)]),
            Progress::Stopped
            | Progress::Booting { .. }
            | Progress::Sealing { seal: Some(_), .. }
            | Progress::Halting {
                requested: true, ..
            } => Plan::Converged,
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::Digest;
    use crate::testing::Scenario;

    type S = Scenario<Workspace>;

    fn id<T: Prefixed>(n: u128) -> Id<T> {
        Id::from_uuid(Uuid::from_u128(n))
    }

    fn snapshot(n: u8) -> SnapshotId {
        SnapshotId::from(Digest::from_blake3([n; 32]))
    }

    fn created() -> WorkspaceEvent {
        WorkspaceEvent::Created {
            id: S::ID,
            spec: WorkspaceSpec {
                owner: id(1),
                repo: id(2),
                branch: "main".parse().expect("branch"),
                desired: Desired::Running,
            },
            at: S::NOW,
        }
    }

    fn booting() -> Vec<WorkspaceEvent> {
        vec![
            created(),
            WorkspaceEvent::SandboxCreated {
                sandbox: id(10),
                secrets: BTreeSet::new(),
            },
        ]
    }

    fn running() -> Vec<WorkspaceEvent> {
        let mut events = booting();
        events.push(WorkspaceEvent::SandboxRunning {
            sandbox: id(10),
            at: S::NOW,
        });
        events
    }

    fn sealing() -> Vec<WorkspaceEvent> {
        let mut events = running();
        events.push(WorkspaceEvent::StopRequested {
            generation: Generation::INITIAL.next(),
        });
        events
    }

    fn halting() -> Vec<WorkspaceEvent> {
        let mut events = sealing();
        events.extend([
            WorkspaceEvent::Sealing { seal: id(30) },
            WorkspaceEvent::Sealed {
                snapshot: snapshot(2),
            },
        ]);
        events
    }

    fn stopped() -> Vec<WorkspaceEvent> {
        let mut events = halting();
        events.extend([
            WorkspaceEvent::SandboxStopRequested,
            WorkspaceEvent::SandboxEnded { sandbox: id(10) },
        ]);
        events
    }

    fn phase(scenario: &S) -> WorkspacePhase {
        scenario.state().status().phase()
    }

    #[test]
    fn a_workspace_opens_runs_seals_and_stops() {
        let opening = S::given([created()]);
        assert_eq!(phase(&opening), WorkspacePhase::Starting);
        opening.plan().then_actions([WorkspaceAction::Open]);

        let booting = opening
            .try_when(|workspace, _| workspace.sandbox_created(id(10), BTreeSet::new()))
            .then([WorkspaceEvent::SandboxCreated {
                sandbox: id(10),
                secrets: BTreeSet::new(),
            }]);
        assert_eq!(phase(&booting), WorkspacePhase::Starting);
        assert_eq!(booting.state().status().sandbox(), Some(id(10)));
        booting.plan().then_converged();

        let running = booting
            .when(|workspace, now| workspace.sandbox_running(id(10), now))
            .then([WorkspaceEvent::SandboxRunning {
                sandbox: id(10),
                at: S::NOW,
            }]);
        assert_eq!(phase(&running), WorkspacePhase::Running);

        let sealing =
            running
                .when(|workspace, _| workspace.stop())
                .then([WorkspaceEvent::StopRequested {
                    generation: Generation::INITIAL.next(),
                }]);
        assert_eq!(phase(&sealing), WorkspacePhase::Stopping);
        sealing.plan().then_actions([WorkspaceAction::Seal(id(10))]);

        let sealed = sealing
            .try_when(|workspace, _| workspace.sealing(id(30)))
            .then([WorkspaceEvent::Sealing { seal: id(30) }])
            .when(|workspace, _| workspace.seal_ended(id(30), Ok(snapshot(2))))
            .then([WorkspaceEvent::Sealed {
                snapshot: snapshot(2),
            }]);
        assert_eq!(sealed.state().status().snapshot(), Some(snapshot(2)));
        sealed
            .plan()
            .then_actions([WorkspaceAction::StopSandbox(id(10))]);

        let halting = sealed
            .try_when(|workspace, _| workspace.sandbox_stopping())
            .then([WorkspaceEvent::SandboxStopRequested]);
        halting.plan().then_converged();

        let stopped = halting
            .when(|workspace, _| workspace.sandbox_ended(id(10)))
            .then([WorkspaceEvent::SandboxEnded { sandbox: id(10) }]);
        assert_eq!(phase(&stopped), WorkspacePhase::Stopped);
        assert_eq!(stopped.state().status().sandbox(), None);
        assert_eq!(stopped.state().status().snapshot(), Some(snapshot(2)));
        stopped.plan().then_converged();
    }

    #[test]
    fn starting_a_stopped_workspace_opens_it_again_and_repeating_a_request_records_nothing() {
        let later = S::NOW.saturating_add(SignedDuration::from_secs(60));
        S::given(stopped())
            .at(later)
            .when(|workspace, now| {
                workspace.start(now).expect("start");
            })
            .then([WorkspaceEvent::StartRequested {
                generation: Generation::INITIAL.next().next(),
                at: later,
            }])
            .plan()
            .then_actions([WorkspaceAction::Open]);
        S::given(running())
            .try_when(Workspace::start)
            .then_no_events();
        S::given(stopped())
            .when(|workspace, _| workspace.stop())
            .then_no_events();
    }

    #[test]
    fn a_start_during_a_stop_waits_for_the_stop_and_then_opens_again() {
        let mut events = sealing();
        events.push(WorkspaceEvent::StartRequested {
            generation: Generation::INITIAL.next().next(),
            at: S::NOW,
        });
        let sealing = S::given(events);
        assert_eq!(phase(&sealing), WorkspacePhase::Stopping);
        sealing.plan().then_actions([WorkspaceAction::Seal(id(10))]);
        let mut events = halting();
        events.extend([
            WorkspaceEvent::StartRequested {
                generation: Generation::INITIAL.next().next(),
                at: S::NOW,
            },
            WorkspaceEvent::SandboxStopRequested,
        ]);
        S::given(events)
            .when(|workspace, _| workspace.sandbox_ended(id(10)))
            .then([WorkspaceEvent::SandboxEnded { sandbox: id(10) }])
            .plan()
            .then_actions([WorkspaceAction::Open]);
    }

    #[test]
    fn stopping_before_the_sandbox_runs_skips_the_seal() {
        let opening = S::given([created()])
            .when(|workspace, _| workspace.stop())
            .then([WorkspaceEvent::StopRequested {
                generation: Generation::INITIAL.next(),
            }]);
        assert_eq!(phase(&opening), WorkspacePhase::Stopped);
        opening.plan().then_converged();

        let booting = S::given(booting())
            .when(|workspace, _| workspace.stop())
            .then([WorkspaceEvent::StopRequested {
                generation: Generation::INITIAL.next(),
            }]);
        booting
            .plan()
            .then_actions([WorkspaceAction::StopSandbox(id(10))]);
        booting
            .when(|workspace, now| workspace.sandbox_running(id(10), now))
            .then_no_events();
    }

    #[test]
    fn a_sandbox_that_ends_by_itself_leaves_the_workspace_stopped_with_its_old_snapshot() {
        let lost = S::given(running())
            .when(|workspace, _| workspace.sandbox_ended(id(10)))
            .then([
                WorkspaceEvent::StopRequested {
                    generation: Generation::INITIAL.next(),
                },
                WorkspaceEvent::SandboxEnded { sandbox: id(10) },
            ]);
        assert_eq!(phase(&lost), WorkspacePhase::Stopped);
        assert_eq!(lost.state().spec().desired, Desired::Stopped);
        lost.plan().then_converged();
        S::given(booting())
            .when(|workspace, _| workspace.sandbox_ended(id(99)))
            .then_no_events();
    }

    #[test]
    fn a_failed_seal_still_stops_the_sandbox_and_keeps_the_previous_snapshot() {
        let mut events = sealing();
        events.push(WorkspaceEvent::Sealing { seal: id(30) });
        S::given(events)
            .when(|workspace, _| workspace.seal_ended(id(31), Ok(snapshot(3))))
            .then_no_events()
            .when(|workspace, _| workspace.seal_ended(id(30), Err(())))
            .then([WorkspaceEvent::SealFailed])
            .plan()
            .then_actions([WorkspaceAction::StopSandbox(id(10))]);
    }

    #[test]
    fn a_sandbox_ending_during_its_seal_waits_for_the_seal() {
        let mut events = sealing();
        events.push(WorkspaceEvent::Sealing { seal: id(30) });
        let ended = S::given(events)
            .when(|workspace, _| workspace.sandbox_ended(id(10)))
            .then([WorkspaceEvent::SandboxEnded { sandbox: id(10) }]);
        assert_eq!(phase(&ended), WorkspacePhase::Stopping);
        let stopped = ended
            .when(|workspace, _| workspace.seal_ended(id(30), Ok(snapshot(2))))
            .then([WorkspaceEvent::Sealed {
                snapshot: snapshot(2),
            }]);
        assert_eq!(phase(&stopped), WorkspacePhase::Stopped);
        assert_eq!(stopped.state().status().snapshot(), Some(snapshot(2)));
    }

    #[test]
    fn steps_out_of_order_are_rejected() {
        S::given(running())
            .try_when(|workspace, _| workspace.sandbox_created(id(11), BTreeSet::new()))
            .then_error("workspace.out_of_order");
        S::given(running())
            .try_when(|workspace, _| workspace.sealing(id(30)))
            .then_error("workspace.out_of_order");
        S::given(sealing())
            .try_when(|workspace, _| workspace.sandbox_stopping())
            .then_error("workspace.out_of_order");
    }

    #[test]
    fn activity_is_recorded_only_while_running_and_idleness_stops_the_workspace() {
        let touched = S::NOW.saturating_add(SignedDuration::from_mins(90));
        let active = S::given(running())
            .at(touched)
            .when(Workspace::touch)
            .then([WorkspaceEvent::ActivityRecorded { at: touched }]);
        let after_two_hours = S::NOW.saturating_add(SignedDuration::from_hours(2));
        active
            .at(after_two_hours)
            .plan()
            .then_recheck(SignedDuration::from_mins(90));
        S::given(running())
            .at(after_two_hours)
            .plan()
            .then_actions([WorkspaceAction::StopIdle]);
        S::given(running())
            .after(SignedDuration::from_mins(119))
            .when(Workspace::stop_if_idle)
            .then_no_events();
        S::given(running())
            .at(after_two_hours)
            .when(Workspace::stop_if_idle)
            .then([WorkspaceEvent::StopRequested {
                generation: Generation::INITIAL.next(),
            }]);
        S::given(booting()).when(Workspace::touch).then_no_events();
        S::given(stopped())
            .at(after_two_hours)
            .when(Workspace::stop_if_idle)
            .then_no_events();
    }

    #[test]
    fn a_deleted_workspace_stops_without_sealing_and_never_starts_again() {
        let deleted = S::given(running())
            .when(Workspace::delete)
            .then([WorkspaceEvent::Deleted {
                generation: Generation::INITIAL.next(),
                at: S::NOW,
            }]);
        assert!(deleted.state().is_deleted());
        deleted
            .plan()
            .then_actions([WorkspaceAction::StopSandbox(id(10))]);
        deleted
            .try_when(Workspace::start)
            .then_error("workspace.deleted")
            .when(Workspace::delete)
            .then_no_events();
        let gone = S::given([created()])
            .when(Workspace::delete)
            .then([WorkspaceEvent::Deleted {
                generation: Generation::INITIAL.next(),
                at: S::NOW,
            }]);
        assert_eq!(phase(&gone), WorkspacePhase::Stopped);
    }

    fn failed_setup() -> WorkspaceEvent {
        WorkspaceEvent::SetupRecorded {
            sandbox: id(10),
            job: id(40),
            outcome: SetupOutcome::Failed {
                reason: "exited with code 1".to_owned(),
            },
        }
    }

    #[test]
    fn the_setup_outcome_of_the_current_sandbox_is_recorded_once() {
        let recorded = S::given(running())
            .when(|workspace, _| {
                workspace.setup_ended(id(10), id(40), SetupOutcome::Succeeded);
            })
            .then([WorkspaceEvent::SetupRecorded {
                sandbox: id(10),
                job: id(40),
                outcome: SetupOutcome::Succeeded,
            }]);
        assert_eq!(
            recorded.state().status().setup(),
            Some(&SetupResult {
                job: id(40),
                outcome: SetupOutcome::Succeeded,
            })
        );
        recorded
            .when(|workspace, _| workspace.setup_ended(id(10), id(40), SetupOutcome::Succeeded))
            .then_no_events();
    }

    #[test]
    fn a_failed_setup_is_visible_and_the_workspace_stays_usable() {
        let failed = S::given(running())
            .when(|workspace, _| {
                workspace.setup_ended(
                    id(10),
                    id(40),
                    SetupOutcome::Failed {
                        reason: "exited with code 1".to_owned(),
                    },
                );
            })
            .then([failed_setup()]);
        assert_eq!(phase(&failed), WorkspacePhase::Running);
        assert!(matches!(
            failed.state().status().setup(),
            Some(SetupResult {
                outcome: SetupOutcome::Failed { .. },
                ..
            })
        ));
        failed.plan().then_recheck(SignedDuration::from_hours(2));
    }

    #[test]
    fn the_setup_of_another_sandbox_is_ignored_and_a_sandbox_without_a_seal_clears_the_last() {
        S::given(running())
            .when(|workspace, _| workspace.setup_ended(id(99), id(40), SetupOutcome::Succeeded))
            .then_no_events();
        S::given(vec![created()])
            .when(|workspace, _| workspace.setup_ended(id(10), id(40), SetupOutcome::Succeeded))
            .then_no_events();

        let mut events = running();
        events.extend([
            failed_setup(),
            WorkspaceEvent::StopRequested {
                generation: Generation::INITIAL.next(),
            },
            WorkspaceEvent::Sealing { seal: id(30) },
            WorkspaceEvent::SealFailed,
            WorkspaceEvent::SandboxStopRequested,
            WorkspaceEvent::SandboxEnded { sandbox: id(10) },
        ]);
        let stopped = S::given(events.clone());
        assert!(
            stopped.state().status().setup().is_some(),
            "kept while the workspace is stopped"
        );
        events.extend([
            WorkspaceEvent::StartRequested {
                generation: Generation::INITIAL.next().next(),
                at: S::NOW,
            },
            WorkspaceEvent::SandboxCreated {
                sandbox: id(11),
                secrets: BTreeSet::new(),
            },
        ]);
        let reopened = S::given(events);
        assert_eq!(
            reopened.state().status().setup(),
            None,
            "nothing was sealed, so the new sandbox is set up again"
        );
    }

    #[test]
    fn events_serialize_stably() {
        let mut events = stopped();
        events.push(failed_setup());
        events.push(WorkspaceEvent::SealFailed);
        events.push(WorkspaceEvent::ActivityRecorded { at: S::NOW });
        insta::assert_json_snapshot!(events);
    }
}
