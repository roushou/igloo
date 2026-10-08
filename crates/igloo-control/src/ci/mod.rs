//! CI: runs a repository's pipeline on every revision of its changes.

mod commands;
mod environment;
mod merge;
mod outcome;
mod outcomes;
mod pipeline;
mod reactors;
mod run;
mod runner;

use std::sync::Arc;

use igloo_core::build::Build;
use igloo_core::change::Change;
use igloo_core::repo::Repo;

use self::commands::StartRunHandler;
pub use self::commands::{
    FailRun, RecordCheckedOut, RecordChecksStarted, RecordPrepared, RecordRunJob,
    RecordRunSandboxStopped, RecordWarmBuilding, RecordWarmBuilt, RunQueries, StartRun,
};
pub use self::environment::{Environment, Environments};
pub use self::merge::{MergeError, Merger};
pub use self::outcome::{
    AgentWork, CheckResult, Outcome, OutcomeEvent, OutcomeId, OutcomeRecord, Verdict,
};
pub use self::outcomes::{AttributeOutcome, OutcomeQueries, RecordOutcome, RecordRevert};
use self::outcomes::{RecordOutcomeHandler, RecordOutcomes, RecordReverts};
pub use self::pipeline::{Base, CheckSpec, Pipeline, SandboxSettings, Warm};
use self::reactors::{RecordRunBuilds, RecordRunJobs, RunEveryRevision};
pub use self::run::{
    Check, CheckOutcome, Planned, Run, RunAction, RunError, RunEvent, RunId, RunOutcome, RunPhase,
    WarmBuild,
};
use self::runner::Runner;
use crate::app::{ControllerSettings, EntityHandler, Extension, InstallError, PlatformBuilder};
use crate::platform::{
    BuildQueries, ChangeQueries, ImageImporter, RepoQueries, RepoSnapshots, Snapshots,
};
use crate::ports::EntityStore;

/// CI: a run per revision, its controller, and the reactors feeding it job and seal outcomes.
pub struct CiModule {
    /// Pacing of the run controller.
    pub settings: ControllerSettings,
}

impl Extension for CiModule {
    fn name(&self) -> &'static str {
        "ci"
    }

    fn install(self: Box<Self>, platform: &mut PlatformBuilder) -> Result<(), InstallError> {
        let store = platform.store::<Run>()?;
        let queries = RunQueries::new(Arc::clone(&store));
        Self::run_commands(platform, &store, &queries)?;
        Self::runner(platform, self.settings, &queries)?;
        Self::outcomes(platform, queries)
    }
}

impl CiModule {
    /// The commands recording each step of a run.
    fn run_commands(
        platform: &mut PlatformBuilder,
        store: &Arc<dyn EntityStore<Run>>,
        queries: &RunQueries,
    ) -> Result<(), InstallError> {
        let ports = platform.ports().clone();
        let clock = &ports.clock;
        platform.command(StartRunHandler {
            queries: queries.clone(),
            store: Arc::clone(store),
            clock: Arc::clone(clock),
            ids: Arc::clone(&ports.ids),
        })?;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordPrepared| command.run,
            |run: &mut Run, command: &RecordPrepared, _| -> Result<(), RunError> {
                run.prepared(command.plan.clone(), command.snapshot, command.warm.clone())
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordWarmBuilding| command.run,
            |run: &mut Run, command: &RecordWarmBuilding, _| -> Result<(), RunError> {
                run.warm_building(command.build)
            },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordWarmBuilt| command.run,
            |run: &mut Run, command: &RecordWarmBuilt, _| {
                run.warm_built(command.build, command.warm.clone());
            },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordRunJob| command.run,
            |run: &mut Run, command: &RecordRunJob, _| {
                run.job_ended(command.job, command.outcome.clone());
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordCheckedOut| command.run,
            |run: &mut Run, command: &RecordCheckedOut, _| -> Result<(), RunError> {
                run.checked_out(command.snapshot)
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordChecksStarted| command.run,
            |run: &mut Run, command: &RecordChecksStarted, _| -> Result<(), RunError> {
                run.checks_started(command.sandbox, command.jobs.clone())
            },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &FailRun| command.run,
            |run: &mut Run, command: &FailRun, _| run.error(command.reason.clone()),
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordRunSandboxStopped| command.run,
            |run: &mut Run, command: &RecordRunSandboxStopped, _| {
                run.sandbox_stopped(command.sandbox);
            },
        ))?;
        Ok(())
    }

    /// The run controller and the reactors feeding it.
    fn runner(
        platform: &mut PlatformBuilder,
        settings: ControllerSettings,
        queries: &RunQueries,
    ) -> Result<(), InstallError> {
        let ports = platform.ports().clone();
        let repos = RepoQueries::new(platform.store::<Repo>()?, Arc::clone(&ports.secrets));
        let runner = Runner {
            bus: platform.bus(),
            ids: Arc::clone(&ports.ids),
            repos,
            checkouts: RepoSnapshots::new(Arc::clone(&ports.forge), Arc::clone(&ports.blobs)),
            environments: Self::environments(platform),
        };
        platform.controller(runner, settings)?;
        platform.reactor(RunEveryRevision {
            changes: ChangeQueries::new(platform.store::<Change>()?),
        });
        platform.reactor(RecordRunJobs {
            runs: queries.clone(),
        });
        platform.reactor(RecordRunBuilds {
            runs: queries.clone(),
            builds: BuildQueries::new(platform.store::<Build>()?),
        });
        Ok(())
    }

    /// Pipelines and base images, as CI and agents read them.
    #[must_use]
    pub fn environments(platform: &PlatformBuilder) -> Environments {
        let ports = platform.ports();
        Environments::new(
            Arc::clone(&ports.forge),
            ImageImporter::new(Arc::clone(&ports.registry), Arc::clone(&ports.blobs)),
            Snapshots::new(Arc::clone(&ports.blobs)),
            platform.bus(),
            Arc::clone(&ports.ids),
        )
    }

    /// Outcomes of ended changes and the reverts of merged ones.
    fn outcomes(platform: &mut PlatformBuilder, queries: RunQueries) -> Result<(), InstallError> {
        let ports = platform.ports().clone();
        let clock = &ports.clock;
        let outcomes = platform.store::<Outcome>()?;
        let outcome_queries = OutcomeQueries::new(Arc::clone(&outcomes));
        platform.command(RecordOutcomeHandler {
            queries: outcome_queries.clone(),
            store: Arc::clone(&outcomes),
            clock: Arc::clone(clock),
            ids: Arc::clone(&ports.ids),
        })?;
        platform.command(EntityHandler::infallible(
            Arc::clone(&outcomes),
            Arc::clone(clock),
            |command: &AttributeOutcome| command.outcome,
            |outcome: &mut Outcome, command: &AttributeOutcome, _| {
                outcome.attribute(command.work.clone());
            },
        ))?;
        platform.command(EntityHandler::infallible(
            outcomes,
            Arc::clone(clock),
            |command: &RecordRevert| command.outcome,
            |outcome: &mut Outcome, command: &RecordRevert, _| {
                outcome.revert(command.commit.clone());
            },
        ))?;
        let changes = ChangeQueries::new(platform.store::<Change>()?);
        platform.reactor(RecordOutcomes {
            changes: changes.clone(),
            runs: queries,
            forge: Arc::clone(&ports.forge),
        });
        platform.reactor(RecordReverts {
            changes,
            repos: RepoQueries::new(platform.store::<Repo>()?, Arc::clone(&ports.secrets)),
            outcomes: outcome_queries,
            forge: Arc::clone(&ports.forge),
        });
        Ok(())
    }
}
