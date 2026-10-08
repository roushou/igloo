//! Builds: a command run in a sandbox over a snapshot, sealed into a new snapshot that is
//! recorded on a repository under a key.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::job::{JobEnding, JobId, JobTimeout};
use crate::repo::{CommitId, RepoId, SecretName};
use crate::sandbox::{SandboxId, SandboxSpec};
use crate::seal::SealId;
use crate::snapshot::SnapshotId;
use crate::{
    Digest, Entity, ErrorCode, Event, Generation, Id, Labels, Plan, Prefixed, Resource, Timestamp,
};

/// Identifies a build (`bld_...`).
pub type BuildId = Id<Build>;

/// A snapshot built by running a command over another: a sandbox from the base, the command,
/// a seal of the sandbox, and the result recorded on the repository under the build's key.
///
/// Invariant: a build ends once, built or failed; it stops its sandbox once it ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Build {
    id: BuildId,
    spec: BuildSpec,
    progress: Progress,
    sandbox: Option<SandboxId>,
    job: Option<JobId>,
    stopped: bool,
    outcome: Option<BuildOutcome>,
    requested_at: Timestamp,
    events: Vec<BuildEvent>,
}

/// What a build does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildSpec {
    /// The repository the snapshot is recorded on.
    pub repo: RepoId,
    /// The key it is recorded under.
    pub key: Digest,
    /// The commit the base snapshot's checkout is at.
    pub commit: CommitId,
    /// The sandbox the command runs in; its snapshot is the base.
    pub sandbox: SandboxSpec,
    /// The shell command.
    pub command: String,
    /// Repository secrets the command gets.
    pub secrets: BTreeSet<SecretName>,
    /// How long the command may run.
    pub timeout: JobTimeout,
}

/// How a build ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BuildOutcome {
    /// The snapshot is built and recorded on the repository.
    Built {
        /// The snapshot.
        snapshot: SnapshotId,
    },
    /// The build could not complete.
    Failed {
        /// Why.
        reason: String,
    },
}

/// Where a build is, between its steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Progress {
    /// Waiting for its sandbox and job.
    Pending,
    /// The command runs.
    Running { job: JobId },
    /// The command succeeded; waiting for the seal.
    Ran,
    /// Sealing.
    Sealing { seal: SealId },
    /// Sealed; waiting to be recorded on the repository.
    Sealed { snapshot: SnapshotId },
    /// Recorded, or failed.
    Ended,
}

/// What the build controller may be asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildAction {
    /// Create the sandbox and submit the command.
    Start,
    /// Seal the sandbox.
    Seal(SandboxId),
    /// Record the snapshot on the repository under the build's key.
    Record(SnapshotId),
    /// Stop the sandbox.
    StopSandbox(SandboxId),
}

/// Facts about a build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BuildEvent {
    /// A build was requested.
    Requested {
        /// Its id.
        id: BuildId,
        /// What it does.
        spec: Box<BuildSpec>,
        /// When.
        at: Timestamp,
    },
    /// Its sandbox was created and its command submitted.
    Started {
        /// The sandbox.
        sandbox: SandboxId,
        /// The command's job.
        job: JobId,
    },
    /// The command ended.
    Ran {
        /// How.
        ending: JobEnding,
    },
    /// The sandbox is being sealed.
    Sealing {
        /// The seal.
        seal: SealId,
    },
    /// The sandbox was sealed.
    Sealed {
        /// The snapshot.
        snapshot: SnapshotId,
    },
    /// The snapshot was recorded on the repository. Final.
    Recorded,
    /// The build could not complete. Final.
    Failed {
        /// Why.
        reason: String,
    },
    /// The sandbox was stopped.
    SandboxStopped,
}

/// Why a build change is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// The change does not apply to the build's state.
    #[error("the build is not at that step")]
    OutOfOrder,
}

impl ErrorCode for BuildError {
    fn code(&self) -> &'static str {
        match self {
            Self::OutOfOrder => "build.out_of_order",
        }
    }
}

impl Build {
    /// A build of `spec`, requested at `now`.
    #[must_use]
    pub fn new(id: BuildId, spec: BuildSpec, now: Timestamp) -> Self {
        let mut build = Self::initial(id, spec.clone(), now);
        build.events.push(BuildEvent::Requested {
            id,
            spec: Box::new(spec),
            at: now,
        });
        build
    }

    /// Records the build's sandbox and the job running its command.
    pub fn started(&mut self, sandbox: SandboxId, job: JobId) -> Result<(), BuildError> {
        if self.progress != Progress::Pending {
            return Err(BuildError::OutOfOrder);
        }
        self.record(BuildEvent::Started { sandbox, job });
        Ok(())
    }

    /// Records how a job ended; only the command's job counts, once. A command that did not
    /// exit 0 fails the build.
    pub fn job_ended(&mut self, job: JobId, ending: JobEnding) {
        if self.progress != (Progress::Running { job }) {
            return;
        }
        self.record(BuildEvent::Ran { ending });
        if !ending.succeeded() {
            self.record(BuildEvent::Failed {
                reason: format!("the build {ending}"),
            });
        }
    }

    /// Records the sandbox's seal.
    pub fn sealing(&mut self, seal: SealId) -> Result<(), BuildError> {
        if self.progress != Progress::Ran {
            return Err(BuildError::OutOfOrder);
        }
        self.record(BuildEvent::Sealing { seal });
        Ok(())
    }

    /// Records how a seal ended: the snapshot, or why it failed. Other seals are ignored.
    pub fn seal_ended(&mut self, seal: SealId, snapshot: Result<SnapshotId, String>) {
        if self.progress != (Progress::Sealing { seal }) {
            return;
        }
        match snapshot {
            Ok(snapshot) => self.record(BuildEvent::Sealed { snapshot }),
            Err(reason) => self.record(BuildEvent::Failed {
                reason: format!("sealing the build failed: {reason}"),
            }),
        }
    }

    /// Records that the snapshot is on the repository; the build is built.
    pub fn recorded(&mut self) -> Result<(), BuildError> {
        if !matches!(self.progress, Progress::Sealed { .. }) {
            return Err(BuildError::OutOfOrder);
        }
        self.record(BuildEvent::Recorded);
        Ok(())
    }

    /// Fails the build, unless it already ended.
    pub fn fail(&mut self, reason: String) {
        if self.outcome.is_none() {
            self.record(BuildEvent::Failed { reason });
        }
    }

    /// Records that the sandbox was stopped.
    pub fn sandbox_stopped(&mut self) {
        if self.sandbox.is_some() && !self.stopped {
            self.record(BuildEvent::SandboxStopped);
        }
    }

    /// What the build does.
    #[must_use]
    pub const fn spec(&self) -> &BuildSpec {
        &self.spec
    }

    /// The job running the command, once started; its logs are the build's output.
    #[must_use]
    pub const fn job(&self) -> Option<JobId> {
        self.job
    }

    /// The seal, while sealing.
    #[must_use]
    pub const fn seal(&self) -> Option<SealId> {
        match self.progress {
            Progress::Sealing { seal } => Some(seal),
            _ => None,
        }
    }

    /// How it ended, once it has.
    #[must_use]
    pub const fn outcome(&self) -> Option<&BuildOutcome> {
        self.outcome.as_ref()
    }

    /// When it was requested.
    #[must_use]
    pub const fn requested_at(&self) -> Timestamp {
        self.requested_at
    }

    const fn initial(id: BuildId, spec: BuildSpec, at: Timestamp) -> Self {
        Self {
            id,
            spec,
            progress: Progress::Pending,
            sandbox: None,
            job: None,
            stopped: false,
            outcome: None,
            requested_at: at,
            events: Vec::new(),
        }
    }

    fn record(&mut self, event: BuildEvent) {
        self.apply(&event);
        self.events.push(event);
    }
}

impl Prefixed for Build {
    const PREFIX: &'static str = "bld";
}

impl Event for BuildEvent {
    const SCHEMA_VERSION: u16 = 1;

    fn kind(&self) -> &'static str {
        match self {
            Self::Requested { .. } => "igloo.build.requested",
            Self::Started { .. } => "igloo.build.started",
            Self::Ran { .. } => "igloo.build.ran",
            Self::Sealing { .. } => "igloo.build.sealing",
            Self::Sealed { .. } => "igloo.build.sealed",
            Self::Recorded => "igloo.build.recorded",
            Self::Failed { .. } => "igloo.build.failed",
            Self::SandboxStopped => "igloo.build.sandbox_stopped",
        }
    }
}

impl Entity for Build {
    const NAME: &'static str = "build";

    type Event = BuildEvent;

    fn id(&self) -> BuildId {
        self.id
    }

    fn from_created(event: &BuildEvent) -> Option<Self> {
        let BuildEvent::Requested { id, spec, at } = event else {
            return None;
        };
        Some(Self::initial(*id, spec.as_ref().clone(), *at))
    }

    fn apply(&mut self, event: &BuildEvent) {
        match event {
            BuildEvent::Requested { .. } => {}
            BuildEvent::Started { sandbox, job } => {
                self.sandbox = Some(*sandbox);
                self.job = Some(*job);
                self.progress = Progress::Running { job: *job };
            }
            BuildEvent::Ran { ending } => {
                if ending.succeeded() {
                    self.progress = Progress::Ran;
                }
            }
            BuildEvent::Sealing { seal } => self.progress = Progress::Sealing { seal: *seal },
            BuildEvent::Sealed { snapshot } => {
                self.progress = Progress::Sealed {
                    snapshot: *snapshot,
                };
            }
            BuildEvent::Recorded => {
                if let Progress::Sealed { snapshot } = self.progress {
                    self.outcome = Some(BuildOutcome::Built { snapshot });
                }
                self.progress = Progress::Ended;
            }
            BuildEvent::Failed { reason } => {
                self.outcome = Some(BuildOutcome::Failed {
                    reason: reason.clone(),
                });
                self.progress = Progress::Ended;
            }
            BuildEvent::SandboxStopped => self.stopped = true,
        }
    }

    fn take_events(&mut self) -> Vec<BuildEvent> {
        std::mem::take(&mut self.events)
    }
}

/// Builds carry no labels.
static NO_LABELS: Labels = Labels::EMPTY;

impl Resource for Build {
    type Spec = BuildSpec;
    type Status = Option<BuildOutcome>;
    type Action = BuildAction;

    fn spec(&self) -> &BuildSpec {
        &self.spec
    }

    fn status(&self) -> &Option<BuildOutcome> {
        &self.outcome
    }

    fn labels(&self) -> &Labels {
        &NO_LABELS
    }

    /// A build's spec never changes.
    fn generation(&self) -> Generation {
        Generation::INITIAL
    }

    /// One step at a time; steps waiting on the job or the seal resume when its outcome is
    /// recorded. An ended build stops its sandbox.
    fn plan(&self, _now: Timestamp) -> Plan<BuildAction> {
        match self.progress {
            Progress::Pending => Plan::Act(vec![BuildAction::Start]),
            Progress::Ran => match self.sandbox {
                Some(sandbox) => Plan::Act(vec![BuildAction::Seal(sandbox)]),
                None => Plan::Converged,
            },
            Progress::Sealed { snapshot } => Plan::Act(vec![BuildAction::Record(snapshot)]),
            Progress::Ended => match self.sandbox {
                Some(sandbox) if !self.stopped => {
                    Plan::Act(vec![BuildAction::StopSandbox(sandbox)])
                }
                _ => Plan::Converged,
            },
            Progress::Running { .. } | Progress::Sealing { .. } => Plan::Converged,
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::job::JobFailure;
    use crate::testing::Scenario;

    type S = Scenario<Build>;

    fn id<T: Prefixed>(n: u128) -> Id<T> {
        Id::from_uuid(Uuid::from_u128(n))
    }

    fn snapshot(n: u8) -> SnapshotId {
        SnapshotId::from(Digest::from_blake3([n; 32]))
    }

    fn requested() -> BuildEvent {
        BuildEvent::Requested {
            id: S::ID,
            spec: Box::new(BuildSpec {
                repo: id(1),
                key: Digest::from_blake3([7; 32]),
                commit: "a".repeat(40).parse().expect("commit"),
                sandbox: SandboxSpec::builder().snapshot(snapshot(1)).build(),
                command: "make deps".to_owned(),
                secrets: BTreeSet::new(),
                timeout: JobTimeout::try_from(jiff::SignedDuration::from_secs(60))
                    .expect("timeout"),
            }),
            at: S::NOW,
        }
    }

    fn started() -> [BuildEvent; 2] {
        [
            requested(),
            BuildEvent::Started {
                sandbox: id(10),
                job: id(20),
            },
        ]
    }

    #[test]
    fn a_build_runs_seals_records_and_stops_its_sandbox() {
        S::given([requested()])
            .plan()
            .then_actions([BuildAction::Start]);
        S::given(started()).plan().then_converged();
        let ran = S::given(started())
            .when(|build, _| build.job_ended(id(99), JobEnding::Exited { code: 0 }))
            .then_no_events()
            .when(|build, _| build.job_ended(id(20), JobEnding::Exited { code: 0 }))
            .then([BuildEvent::Ran {
                ending: JobEnding::Exited { code: 0 },
            }]);
        ran.plan().then_actions([BuildAction::Seal(id(10))]);
        let sealed = ran
            .try_when(|build, _| build.sealing(id(30)))
            .then([BuildEvent::Sealing { seal: id(30) }])
            .when(|build, _| build.seal_ended(id(30), Ok(snapshot(2))))
            .then([BuildEvent::Sealed {
                snapshot: snapshot(2),
            }]);
        sealed
            .plan()
            .then_actions([BuildAction::Record(snapshot(2))]);
        let recorded = sealed
            .try_when(|build, _| build.recorded())
            .then([BuildEvent::Recorded]);
        assert_eq!(
            recorded.state().outcome(),
            Some(&BuildOutcome::Built {
                snapshot: snapshot(2)
            })
        );
        recorded
            .plan()
            .then_actions([BuildAction::StopSandbox(id(10))]);
        recorded
            .when(|build, _| build.sandbox_stopped())
            .then([BuildEvent::SandboxStopped])
            .plan()
            .then_converged();
    }

    #[test]
    fn a_failing_command_or_seal_fails_the_build() {
        let failed = S::given(started())
            .when(|build, _| build.job_ended(id(20), JobEnding::Exited { code: 101 }))
            .then([
                BuildEvent::Ran {
                    ending: JobEnding::Exited { code: 101 },
                },
                BuildEvent::Failed {
                    reason: "the build exited with 101".to_owned(),
                },
            ]);
        failed
            .plan()
            .then_actions([BuildAction::StopSandbox(id(10))]);
        let lost = JobEnding::Failed {
            reason: JobFailure::LeaseLost,
        };
        S::given(started())
            .when(|build, _| build.job_ended(id(20), lost))
            .then([
                BuildEvent::Ran { ending: lost },
                BuildEvent::Failed {
                    reason: "the build did not complete: lease lost".to_owned(),
                },
            ]);
        let mut sealing = started().to_vec();
        sealing.extend([
            BuildEvent::Ran {
                ending: JobEnding::Exited { code: 0 },
            },
            BuildEvent::Sealing { seal: id(30) },
        ]);
        S::given(sealing)
            .when(|build, _| build.seal_ended(id(30), Err("the sandbox ended".to_owned())))
            .then([BuildEvent::Failed {
                reason: "sealing the build failed: the sandbox ended".to_owned(),
            }]);
    }

    #[test]
    fn steps_out_of_order_are_rejected_and_an_ended_build_does_not_fail_again() {
        S::given([requested()])
            .try_when(|build, _| build.sealing(id(30)))
            .then_error("build.out_of_order");
        S::given(started())
            .try_when(|build, _| build.recorded())
            .then_error("build.out_of_order");
        S::given(started())
            .when(|build, _| build.fail("stopped".to_owned()))
            .then([BuildEvent::Failed {
                reason: "stopped".to_owned(),
            }])
            .when(|build, _| build.fail("again".to_owned()))
            .then_no_events();
    }

    #[test]
    fn events_serialize_stably() {
        insta::assert_json_snapshot!(started());
    }
}
