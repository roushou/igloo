use async_trait::async_trait;
use igloo_core::build::{Build, BuildEvent, BuildOutcome};
use igloo_core::change::{Change, ChangeEvent};
use igloo_core::job::{Job, JobEvent};
use igloo_core::repo::WarmSnapshot;
use igloo_core::{Actor, Entity, SystemComponent};

use super::commands::{RecordRunJob, RecordWarmBuilt, RunQueries, StartRun};
use super::run::{CheckOutcome, Run, RunEvent};
use crate::app::{AppError, CommandBus, Reactor, RequestContext};
use crate::platform::{BuildQueries, ChangeQueries};
use crate::ports::EventEnvelope;

/// Starts a run for every revision of every change.
pub(super) struct RunEveryRevision {
    pub(super) changes: ChangeQueries,
}

/// Records how the jobs runs wait on ended.
pub(super) struct RecordRunJobs {
    pub(super) runs: RunQueries,
}

/// Records how the builds runs wait on ended, whichever of the build's end and the run's wait
/// is recorded first.
pub(super) struct RecordRunBuilds {
    pub(super) runs: RunQueries,
    pub(super) builds: BuildQueries,
}

fn context(event: &EventEnvelope) -> RequestContext {
    RequestContext::caused_by(
        event,
        Actor::System {
            component: SystemComponent::Reactor,
        },
    )
}

#[async_trait]
impl Reactor for RunEveryRevision {
    fn name(&self) -> &'static str {
        "ci.run_every_revision"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let Some(change) = event.subject_as::<Change>() else {
            return Ok(());
        };
        let revision = match event.decode::<ChangeEvent>()? {
            ChangeEvent::Opened { revision, .. } | ChangeEvent::Revised { revision } => revision,
            ChangeEvent::Closed
            | ChangeEvent::Approved { .. }
            | ChangeEvent::Commented { .. }
            | ChangeEvent::ChangesRequested { .. }
            | ChangeEvent::Merged { .. } => return Ok(()),
        };
        let Some(found) = self.changes.get(change).await? else {
            return Ok(());
        };
        let command = StartRun {
            repo: found.repo(),
            change,
            revision: revision.number,
            commit: revision.head,
        };
        bus.dispatch(command, context(event)).await.map(drop)
    }
}

#[async_trait]
impl Reactor for RecordRunJobs {
    fn name(&self) -> &'static str {
        "ci.record_run_jobs"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let Some(job) = event.subject_as::<Job>() else {
            return Ok(());
        };
        let Some(ending) = event.decode::<JobEvent>()?.ending() else {
            return Ok(());
        };
        let outcome = CheckOutcome::from(ending);
        let Some(run) = self.runs.with_job(job).await? else {
            return Ok(());
        };
        let command = RecordRunJob {
            run: run.id(),
            job,
            outcome,
        };
        bus.dispatch(command, context(event)).await
    }
}

#[async_trait]
impl Reactor for RecordRunBuilds {
    fn name(&self) -> &'static str {
        "ci.record_run_builds"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let (build, runs) = if let Some(build) = event.subject_as::<Build>() {
            if !matches!(
                event.decode::<BuildEvent>()?,
                BuildEvent::Recorded | BuildEvent::Failed { .. }
            ) {
                return Ok(());
            }
            (build, self.runs.of_build(build).await?)
        } else if let Some(run) = event.subject_as::<Run>() {
            let RunEvent::WarmBuilding { build } = event.decode::<RunEvent>()? else {
                return Ok(());
            };
            (build, self.runs.get(run).await?.into_iter().collect())
        } else {
            return Ok(());
        };
        let Some(found) = self.builds.get(build).await? else {
            return Ok(());
        };
        let warm = match found.outcome() {
            Some(BuildOutcome::Built { snapshot }) => Ok(WarmSnapshot {
                snapshot: *snapshot,
                commit: found.spec().commit.clone(),
            }),
            Some(BuildOutcome::Failed { reason }) => Err(reason.clone()),
            None => return Ok(()),
        };
        for run in runs {
            let command = RecordWarmBuilt {
                run: run.id(),
                build,
                warm: warm.clone(),
            };
            bus.dispatch(command, context(event)).await?;
        }
        Ok(())
    }
}
