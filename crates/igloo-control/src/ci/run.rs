use igloo_core::build::BuildId;
use igloo_core::change::ChangeId;
use igloo_core::job::{JobEnding, JobId};
use igloo_core::repo::{CommitId, RepoId, WarmSnapshot};
use igloo_core::sandbox::SandboxId;
use igloo_core::snapshot::SnapshotId;
use igloo_core::{
    Digest, Entity, ErrorCode, Event, Generation, Id, Labels, Plan, Prefixed, Resource, Timestamp,
};
use serde::{Deserialize, Serialize};

use super::{CheckSpec, SandboxSettings};

/// Identifies a run (`run_...`).
pub type RunId = Id<Run>;

/// The checks of one revision of a change, from its pipeline, in a sandbox forked from a warm
/// snapshot of the revision's commit; the warm snapshot is built first when missing.
///
/// Invariant: a run ends once, `passed`, `failed` or `errored`, and never changes outcome; every
/// sandbox it starts is stopped once it ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    id: RunId,
    repo: RepoId,
    change: ChangeId,
    revision: u32,
    commit: CommitId,
    plan: Option<Planned>,
    warm: WarmProgress,
    warm_build: Option<BuildId>,
    snapshot: Option<SnapshotId>,
    checks: Vec<Check>,
    sandbox: Option<SandboxId>,
    outcome: Option<RunOutcome>,
    stopped: Vec<SandboxId>,
    started_at: Timestamp,
    events: Vec<RunEvent>,
}

/// What preparing a run settled: how its sandbox is set up and what it checks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Planned {
    /// The sandbox settings.
    pub settings: SandboxSettings,
    /// The checks, in pipeline order.
    pub checks: Vec<CheckSpec>,
}

/// A warm snapshot to build before checking.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WarmBuild {
    /// What it is built from.
    pub key: Digest,
    /// The cold checkout it is built in.
    pub snapshot: SnapshotId,
    /// The build command.
    pub command: String,
    /// Whether the build may reach the network.
    pub network: igloo_core::sandbox::NetworkPolicy,
    /// How long the build may run, in seconds.
    pub timeout_seconds: u32,
}

/// Where building the warm snapshot is.
#[derive(Clone, Debug, PartialEq, Eq)]
enum WarmProgress {
    /// Not needed, or not decided yet.
    None,
    /// Waiting for its build to start.
    Needed(WarmBuild),
    /// Built by `build`.
    Building(BuildId),
    /// Built; the checkout goes over it next.
    Built(WarmSnapshot),
}

/// One check of a run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// What it runs.
    pub spec: CheckSpec,
    /// The job running it, once started.
    pub job: Option<JobId>,
    /// How it ended, once it has.
    pub outcome: Option<CheckOutcome>,
}

/// How a check or job ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CheckOutcome {
    /// Exited 0.
    Passed,
    /// Exited with another code.
    Failed {
        /// The code.
        exit_code: i32,
    },
    /// Did not complete: timed out, lost, cancelled, or could not start.
    Errored {
        /// Why.
        reason: String,
    },
}

impl From<JobEnding> for CheckOutcome {
    fn from(ending: JobEnding) -> Self {
        match ending {
            JobEnding::Exited { code: 0 } => Self::Passed,
            JobEnding::Exited { code } => Self::Failed { exit_code: code },
            JobEnding::Failed { reason } => Self::Errored {
                reason: reason.to_string(),
            },
            JobEnding::Cancelled => Self::Errored {
                reason: "cancelled".to_owned(),
            },
        }
    }
}

impl std::fmt::Display for CheckOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Passed => f.write_str("passed"),
            Self::Failed { exit_code } => write!(f, "exited with {exit_code}"),
            Self::Errored { reason } => write!(f, "did not complete: {reason}"),
        }
    }
}

/// How a run ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RunOutcome {
    /// Every check passed.
    Passed,
    /// A check failed or errored.
    Failed,
    /// The run could not check: no pipeline, an invalid one, or a failed warm build.
    Errored {
        /// Why.
        reason: String,
    },
}

/// Where a run is, as reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunPhase {
    /// Reading the pipeline and finding the snapshot.
    Preparing,
    /// Building the warm snapshot.
    Warming,
    /// Running the checks.
    Checking,
    /// Ended; see [`Run::outcome`].
    Ended,
}

/// What the run controller may be asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunAction {
    /// Read the pipeline and decide the snapshot, or the warm build that precedes it.
    Prepare,
    /// Start the warm build over the cold checkout.
    StartWarm(WarmBuild),
    /// Check the revision out over the built warm snapshot.
    CheckOut(WarmSnapshot),
    /// Start a sandbox from the snapshot and submit every check.
    StartChecks(SnapshotId),
    /// Stop sandboxes the run no longer needs.
    StopSandboxes(Vec<SandboxId>),
}

/// Facts about a run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    /// A run of a revision started.
    Started {
        /// Its id.
        id: RunId,
        /// The repository.
        repo: RepoId,
        /// The change.
        change: ChangeId,
        /// The revision's number.
        revision: u32,
        /// The revision's head.
        commit: CommitId,
        /// When.
        at: Timestamp,
    },
    /// The pipeline was read; the run checks `snapshot` or builds `warm` first.
    Prepared {
        /// Sandbox settings and checks.
        plan: Planned,
        /// The snapshot to check, when no warm build is needed.
        snapshot: Option<SnapshotId>,
        /// The warm build to do first.
        warm: Option<WarmBuild>,
    },
    /// The warm snapshot is being built.
    WarmBuilding {
        /// The build.
        build: BuildId,
    },
    /// The warm snapshot was built.
    WarmBuilt {
        /// The snapshot and the commit it was built at.
        warm: WarmSnapshot,
    },
    /// The checkout over the warm snapshot is ready.
    CheckedOut {
        /// The snapshot to check.
        snapshot: SnapshotId,
    },
    /// The checks were submitted in one sandbox.
    ChecksStarted {
        /// The sandbox.
        sandbox: SandboxId,
        /// One job per check, in check order.
        jobs: Vec<JobId>,
    },
    /// A check's job ended.
    CheckEnded {
        /// The job.
        job: JobId,
        /// How.
        outcome: CheckOutcome,
    },
    /// The run cannot check.
    Errored {
        /// Why.
        reason: String,
    },
    /// A sandbox of the run was stopped.
    SandboxStopped {
        /// The sandbox.
        sandbox: SandboxId,
    },
}

/// Why a run change is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RunError {
    /// The change does not apply to the run's state.
    #[error("the run is not at that step")]
    OutOfOrder,
}

impl ErrorCode for RunError {
    fn code(&self) -> &'static str {
        match self {
            Self::OutOfOrder => "run.out_of_order",
        }
    }
}

impl Run {
    /// A run of `change`'s revision `revision`, at `commit`.
    #[must_use]
    pub fn new(
        id: RunId,
        repo: RepoId,
        (change, revision): (ChangeId, u32),
        commit: CommitId,
        now: Timestamp,
    ) -> Self {
        let mut run = Self::initial(id, repo, change, revision, commit.clone(), now);
        run.events.push(RunEvent::Started {
            id,
            repo,
            change,
            revision,
            commit,
            at: now,
        });
        run
    }

    /// Records the prepared pipeline: checks of `snapshot`, or a warm build first.
    pub fn prepared(
        &mut self,
        plan: Planned,
        snapshot: Option<SnapshotId>,
        warm: Option<WarmBuild>,
    ) -> Result<(), RunError> {
        if self.plan.is_some() || self.outcome.is_some() || snapshot.is_some() == warm.is_some() {
            return Err(RunError::OutOfOrder);
        }
        self.record(RunEvent::Prepared {
            plan,
            snapshot,
            warm,
        });
        Ok(())
    }

    /// Records the build making the warm snapshot.
    pub fn warm_building(&mut self, build: BuildId) -> Result<(), RunError> {
        if !matches!(self.warm, WarmProgress::Needed(_)) {
            return Err(RunError::OutOfOrder);
        }
        self.record(RunEvent::WarmBuilding { build });
        Ok(())
    }

    /// Records how the warm build ended: the warm snapshot, or why it failed. Other builds
    /// and repeated reports are ignored.
    pub fn warm_built(&mut self, build: BuildId, warm: Result<WarmSnapshot, String>) {
        if self.warm != WarmProgress::Building(build) || self.outcome.is_some() {
            return;
        }
        match warm {
            Ok(warm) => self.record(RunEvent::WarmBuilt { warm }),
            Err(reason) => self.record(RunEvent::Errored {
                reason: format!("the warm snapshot: {reason}"),
            }),
        }
    }

    /// Records how a check's job ended. Other jobs and repeated reports are ignored.
    pub fn job_ended(&mut self, job: JobId, outcome: CheckOutcome) {
        let pending = self
            .checks
            .iter()
            .any(|check| check.job == Some(job) && check.outcome.is_none());
        if pending {
            self.record(RunEvent::CheckEnded { job, outcome });
        }
    }

    /// Records the checkout over the warm snapshot.
    pub fn checked_out(&mut self, snapshot: SnapshotId) -> Result<(), RunError> {
        if !matches!(self.warm, WarmProgress::Built(_)) || self.snapshot.is_some() {
            return Err(RunError::OutOfOrder);
        }
        self.record(RunEvent::CheckedOut { snapshot });
        Ok(())
    }

    /// Records the checks' sandbox and jobs, one per check in order.
    pub fn checks_started(&mut self, sandbox: SandboxId, jobs: Vec<JobId>) -> Result<(), RunError> {
        if self.snapshot.is_none()
            || self.sandbox.is_some()
            || self.outcome.is_some()
            || jobs.len() != self.checks.len()
        {
            return Err(RunError::OutOfOrder);
        }
        self.record(RunEvent::ChecksStarted { sandbox, jobs });
        Ok(())
    }

    /// Ends the run as errored, unless it already ended.
    pub fn error(&mut self, reason: String) {
        if self.outcome.is_none() {
            self.record(RunEvent::Errored { reason });
        }
    }

    /// Records that `sandbox` was stopped.
    pub fn sandbox_stopped(&mut self, sandbox: SandboxId) {
        if !self.stopped.contains(&sandbox) {
            self.record(RunEvent::SandboxStopped { sandbox });
        }
    }

    /// The repository.
    #[must_use]
    pub const fn repo(&self) -> RepoId {
        self.repo
    }

    /// The change.
    #[must_use]
    pub const fn change(&self) -> ChangeId {
        self.change
    }

    /// The revision's number.
    #[must_use]
    pub const fn revision(&self) -> u32 {
        self.revision
    }

    /// The revision's head.
    #[must_use]
    pub const fn commit(&self) -> &CommitId {
        &self.commit
    }

    /// The checks, in pipeline order; empty until prepared.
    #[must_use]
    pub fn checks(&self) -> &[Check] {
        &self.checks
    }

    /// The sandbox settings, once prepared.
    #[must_use]
    pub fn settings(&self) -> Option<&SandboxSettings> {
        self.plan.as_ref().map(|plan| &plan.settings)
    }

    /// The build making the run's warm snapshot, if the run needed one.
    #[must_use]
    pub const fn warm_build(&self) -> Option<BuildId> {
        self.warm_build
    }

    /// How it ended, once it has.
    #[must_use]
    pub const fn outcome(&self) -> Option<&RunOutcome> {
        self.outcome.as_ref()
    }

    /// Where it is.
    #[must_use]
    pub const fn phase(&self) -> RunPhase {
        if self.outcome.is_some() {
            RunPhase::Ended
        } else if self.sandbox.is_some() || self.snapshot.is_some() {
            RunPhase::Checking
        } else if matches!(self.warm, WarmProgress::None) {
            RunPhase::Preparing
        } else {
            RunPhase::Warming
        }
    }

    /// When it started.
    #[must_use]
    pub const fn started_at(&self) -> Timestamp {
        self.started_at
    }

    /// Every sandbox the run started that is not stopped yet.
    fn running_sandboxes(&self) -> Vec<SandboxId> {
        self.sandbox
            .into_iter()
            .filter(|sandbox| !self.stopped.contains(sandbox))
            .collect()
    }

    fn initial(
        id: RunId,
        repo: RepoId,
        change: ChangeId,
        revision: u32,
        commit: CommitId,
        at: Timestamp,
    ) -> Self {
        Self {
            id,
            repo,
            change,
            revision,
            commit,
            plan: None,
            warm: WarmProgress::None,
            warm_build: None,
            snapshot: None,
            checks: Vec::new(),
            sandbox: None,
            outcome: None,
            stopped: Vec::new(),
            started_at: at,
            events: Vec::new(),
        }
    }

    fn record(&mut self, event: RunEvent) {
        self.apply(&event);
        self.events.push(event);
    }

    /// Ends the run once every check has ended.
    fn conclude(&mut self) {
        if self.checks.iter().all(|check| check.outcome.is_some()) {
            let passed = self
                .checks
                .iter()
                .all(|check| check.outcome == Some(CheckOutcome::Passed));
            self.outcome = Some(if passed {
                RunOutcome::Passed
            } else {
                RunOutcome::Failed
            });
        }
    }
}

impl Prefixed for Run {
    const PREFIX: &'static str = "run";
}

impl Event for RunEvent {
    const SCHEMA_VERSION: u16 = 1;

    fn kind(&self) -> &'static str {
        match self {
            Self::Started { .. } => "igloo.run.started",
            Self::Prepared { .. } => "igloo.run.prepared",
            Self::WarmBuilding { .. } => "igloo.run.warm_building",
            Self::WarmBuilt { .. } => "igloo.run.warm_built",
            Self::CheckedOut { .. } => "igloo.run.checked_out",
            Self::ChecksStarted { .. } => "igloo.run.checks_started",
            Self::CheckEnded { .. } => "igloo.run.check_ended",
            Self::Errored { .. } => "igloo.run.errored",
            Self::SandboxStopped { .. } => "igloo.run.sandbox_stopped",
        }
    }
}

impl Entity for Run {
    const NAME: &'static str = "run";

    type Event = RunEvent;

    fn id(&self) -> RunId {
        self.id
    }

    fn from_created(event: &RunEvent) -> Option<Self> {
        let RunEvent::Started {
            id,
            repo,
            change,
            revision,
            commit,
            at,
        } = event
        else {
            return None;
        };
        Some(Self::initial(
            *id,
            *repo,
            *change,
            *revision,
            commit.clone(),
            *at,
        ))
    }

    fn apply(&mut self, event: &RunEvent) {
        match event {
            RunEvent::Started { .. } => {}
            RunEvent::Prepared {
                plan,
                snapshot,
                warm,
            } => {
                self.checks = plan
                    .checks
                    .iter()
                    .map(|spec| Check {
                        spec: spec.clone(),
                        job: None,
                        outcome: None,
                    })
                    .collect();
                self.plan = Some(plan.clone());
                self.snapshot = *snapshot;
                if let Some(build) = warm {
                    self.warm = WarmProgress::Needed(build.clone());
                }
            }
            RunEvent::WarmBuilding { build } => {
                self.warm_build = Some(*build);
                self.warm = WarmProgress::Building(*build);
            }
            RunEvent::WarmBuilt { warm } => self.warm = WarmProgress::Built(warm.clone()),
            RunEvent::CheckedOut { snapshot } => self.snapshot = Some(*snapshot),
            RunEvent::ChecksStarted { sandbox, jobs } => {
                self.sandbox = Some(*sandbox);
                for (check, job) in self.checks.iter_mut().zip(jobs) {
                    check.job = Some(*job);
                }
            }
            RunEvent::CheckEnded { job, outcome } => {
                if let Some(check) = self.checks.iter_mut().find(|check| check.job == Some(*job)) {
                    check.outcome = Some(outcome.clone());
                }
                self.conclude();
            }
            RunEvent::Errored { reason } => {
                self.outcome = Some(RunOutcome::Errored {
                    reason: reason.clone(),
                });
            }
            RunEvent::SandboxStopped { sandbox } => self.stopped.push(*sandbox),
        }
    }

    fn take_events(&mut self) -> Vec<RunEvent> {
        std::mem::take(&mut self.events)
    }
}

/// Runs carry no labels.
static NO_LABELS: Labels = Labels::EMPTY;

impl Resource for Run {
    type Spec = CommitId;
    type Status = Option<RunOutcome>;
    type Action = RunAction;

    fn spec(&self) -> &CommitId {
        &self.commit
    }

    fn status(&self) -> &Option<RunOutcome> {
        &self.outcome
    }

    fn labels(&self) -> &Labels {
        &NO_LABELS
    }

    /// A run's spec never changes.
    fn generation(&self) -> Generation {
        Generation::INITIAL
    }

    /// One step at a time; steps that wait on jobs and the warm build resume when their
    /// outcome is recorded. An ended run stops its sandbox.
    fn plan(&self, _now: Timestamp) -> Plan<RunAction> {
        if self.outcome.is_some() {
            let running = self.running_sandboxes();
            return if running.is_empty() {
                Plan::Converged
            } else {
                Plan::Act(vec![RunAction::StopSandboxes(running)])
            };
        }
        if self.plan.is_none() {
            return Plan::Act(vec![RunAction::Prepare]);
        }
        match &self.warm {
            WarmProgress::Needed(build) => {
                return Plan::Act(vec![RunAction::StartWarm(build.clone())]);
            }
            WarmProgress::Building(_) => return Plan::Converged,
            WarmProgress::Built(warm) if self.snapshot.is_none() => {
                return Plan::Act(vec![RunAction::CheckOut(warm.clone())]);
            }
            WarmProgress::None | WarmProgress::Built(_) => {}
        }
        if let (Some(snapshot), None) = (self.snapshot, self.sandbox) {
            return Plan::Act(vec![RunAction::StartChecks(snapshot)]);
        }
        Plan::Converged
    }
}

#[cfg(test)]
mod tests {
    use igloo_core::sandbox::{Isolation, NetworkPolicy};
    use igloo_core::testing::Scenario;
    use uuid::Uuid;

    use super::*;
    use crate::ci::SandboxSettings;

    type S = Scenario<Run>;

    fn id<T: Prefixed>(n: u128) -> Id<T> {
        Id::from_uuid(Uuid::from_u128(n))
    }

    fn snapshot(n: u8) -> SnapshotId {
        SnapshotId::from(Digest::from_blake3([n; 32]))
    }

    fn started() -> RunEvent {
        RunEvent::Started {
            id: S::ID,
            repo: id(1),
            change: id(2),
            revision: 1,
            commit: "a".repeat(40).parse().expect("commit"),
            at: S::NOW,
        }
    }

    fn planned() -> Planned {
        let check = |name: &str| CheckSpec {
            name: name.to_owned(),
            run: format!("make {name}"),
            timeout_seconds: 60,
        };
        Planned {
            settings: SandboxSettings {
                isolation: Isolation::Any,
                limits: igloo_core::sandbox::ResourceLimits::default(),
                network: NetworkPolicy::DenyAll,
                env: igloo_core::process::EnvVars::default(),
                secrets: std::collections::BTreeSet::new(),
            },
            checks: vec![check("lint"), check("test")],
        }
    }

    fn build() -> WarmBuild {
        WarmBuild {
            key: Digest::from_blake3([7; 32]),
            snapshot: snapshot(1),
            command: "make deps".to_owned(),
            network: NetworkPolicy::AllowAll,
            timeout_seconds: 600,
        }
    }

    fn prepared_warm() -> RunEvent {
        RunEvent::Prepared {
            plan: planned(),
            snapshot: None,
            warm: Some(build()),
        }
    }

    #[test]
    fn a_new_run_prepares() {
        S::given([started()])
            .plan()
            .then_actions([RunAction::Prepare]);
    }

    fn warm(n: u8) -> WarmSnapshot {
        WarmSnapshot {
            snapshot: snapshot(n),
            commit: "b".repeat(40).parse().expect("commit"),
        }
    }

    #[test]
    fn the_warm_build_comes_before_the_checks() {
        S::given([started(), prepared_warm()])
            .plan()
            .then_actions([RunAction::StartWarm(build())]);
        let building = S::given([started(), prepared_warm()])
            .try_when(|run, _| run.warm_building(id(40)))
            .then([RunEvent::WarmBuilding { build: id(40) }]);
        assert_eq!(building.state().warm_build(), Some(id(40)));
        assert_eq!(building.state().phase(), RunPhase::Warming);
        building.plan().then_converged();
        let built = building
            .when(|run, _| run.warm_built(id(41), Ok(warm(2))))
            .then_no_events()
            .when(|run, _| run.warm_built(id(40), Ok(warm(2))))
            .then([RunEvent::WarmBuilt { warm: warm(2) }]);
        built.plan().then_actions([RunAction::CheckOut(warm(2))]);
        built
            .try_when(|run, _| run.checked_out(snapshot(3)))
            .then([RunEvent::CheckedOut {
                snapshot: snapshot(3),
            }])
            .plan()
            .then_actions([RunAction::StartChecks(snapshot(3))]);
    }

    #[test]
    fn a_failed_warm_build_errors_the_run() {
        let errored = S::given([
            started(),
            prepared_warm(),
            RunEvent::WarmBuilding { build: id(40) },
        ])
        .when(|run, _| run.warm_built(id(40), Err("the build exited with 2".to_owned())))
        .then([RunEvent::Errored {
            reason: "the warm snapshot: the build exited with 2".to_owned(),
        }]);
        assert!(matches!(
            errored.state().outcome(),
            Some(RunOutcome::Errored { .. })
        ));
        errored.plan().then_converged();
        errored
            .when(|run, _| run.warm_built(id(40), Ok(warm(2))))
            .then_no_events();
    }

    #[test]
    fn the_run_ends_failed_when_a_check_fails_and_ignores_other_jobs() {
        let checking = [
            started(),
            RunEvent::Prepared {
                plan: planned(),
                snapshot: Some(snapshot(3)),
                warm: None,
            },
            RunEvent::ChecksStarted {
                sandbox: id(11),
                jobs: vec![id(21), id(22)],
            },
        ];
        let ended = S::given(checking)
            .when(|run, _| run.job_ended(id(99), CheckOutcome::Passed))
            .then_no_events()
            .when(|run, _| run.job_ended(id(21), CheckOutcome::Passed))
            .then([RunEvent::CheckEnded {
                job: id(21),
                outcome: CheckOutcome::Passed,
            }])
            .when(|run, _| run.job_ended(id(22), CheckOutcome::Failed { exit_code: 1 }))
            .then([RunEvent::CheckEnded {
                job: id(22),
                outcome: CheckOutcome::Failed { exit_code: 1 },
            }]);
        assert_eq!(ended.state().outcome(), Some(&RunOutcome::Failed));
        assert_eq!(ended.state().phase(), RunPhase::Ended);
        let stopped = ended
            .when(|run, _| run.sandbox_stopped(id(11)))
            .then([RunEvent::SandboxStopped { sandbox: id(11) }]);
        stopped.plan().then_converged();
    }

    #[test]
    fn a_run_is_prepared_once() {
        S::given([started(), prepared_warm()])
            .try_when(|run, _| run.prepared(planned(), Some(snapshot(3)), None))
            .then_error("run.out_of_order");
    }
}
