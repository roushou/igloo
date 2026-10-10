use std::collections::BTreeSet;

use igloo_core::build::BuildSpec;
use igloo_core::job::{JobId, JobSpec, JobTimeout};
use igloo_core::process::{Argv, EnvVars};
use igloo_core::repo::{Repo, SecretName, WarmSnapshot};
use igloo_core::sandbox::{NetworkPolicy, SandboxId, SandboxSpec};
use igloo_core::snapshot::SnapshotId;
use igloo_core::{Actor, Entity, SystemComponent};
use jiff::SignedDuration;

use super::commands::{
    FailRun, RecordCheckedOut, RecordChecksStarted, RecordPrepared, RecordRunSandboxStopped,
    RecordWarmBuilding,
};
use super::environment::{Environment, Environments};
use super::pipeline::SandboxSettings;
use super::run::{Planned, Run, RunAction, WarmBuild};
use crate::app::{AppError, CommandBus, Reconciler, RequestContext};
use crate::platform::{
    CreateSandbox, RecordWarmUse, RepoQueries, RepoSnapshots, StartBuild, StopSandbox, SubmitJob,
    WarmRecipe,
};
use crate::ports::{IdGenerator, IdGeneratorExt};

/// Carries out runs' plans: prepares their snapshots, builds warm snapshots, starts checks and
/// stops sandboxes, recording each step on the run.
pub(super) struct Runner {
    pub(super) bus: CommandBus,
    pub(super) ids: std::sync::Arc<dyn IdGenerator>,
    pub(super) repos: RepoQueries,
    pub(super) checkouts: RepoSnapshots,
    pub(super) environments: Environments,
}

impl Reconciler<Run> for Runner {
    async fn reconcile(&self, run: &Run, actions: Vec<RunAction>) -> Result<(), AppError> {
        for action in actions {
            match action {
                RunAction::Prepare => self.prepare(run).await?,
                RunAction::StartWarm(build) => self.start_warm(run, &build).await?,
                RunAction::CheckOut(warm) => self.check_out(run, &warm).await?,
                RunAction::StartChecks(snapshot) => self.start_checks(run, snapshot).await?,
                RunAction::StopSandboxes(sandboxes) => {
                    for sandbox in sandboxes {
                        self.dispatch(StopSandbox { sandbox }).await?;
                        self.dispatch(RecordRunSandboxStopped {
                            run: run.id(),
                            sandbox,
                        })
                        .await?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl Runner {
    /// Reads the pipeline at the run's commit and records what to check, or the warm build to do
    /// first. A missing or invalid pipeline ends the run as errored.
    async fn prepare(&self, run: &Run) -> Result<(), AppError> {
        let repo = self.repo(run).await?;
        let Environment { pipeline, base } = match self.environments.at(&repo, run.commit()).await?
        {
            Ok(environment) => environment,
            Err(reason) => return self.fail(run, reason).await,
        };
        let plan = Planned {
            settings: pipeline.sandbox.clone(),
            checks: pipeline.checks.clone(),
        };
        let (snapshot, warm) = match &pipeline.warm {
            None => (
                Some(
                    self.checkouts
                        .checkout(repo.id(), run.commit(), base, None)
                        .await?,
                ),
                None,
            ),
            Some(warm) => {
                let recipe = WarmRecipe {
                    command: warm.command.clone(),
                    lockfiles: warm.lockfiles.clone(),
                };
                let key = self
                    .checkouts
                    .warm_key(repo.id(), run.commit(), base, &recipe)
                    .await?;
                if let Some(built) = repo.warm(&key) {
                    self.dispatch(RecordWarmUse {
                        repo: repo.id(),
                        keys: vec![key],
                    })
                    .await?;
                    let snapshot = self
                        .checkouts
                        .checkout(repo.id(), run.commit(), base, Some(built))
                        .await?;
                    (Some(snapshot), None)
                } else {
                    let cold = self
                        .checkouts
                        .checkout(repo.id(), run.commit(), base, None)
                        .await?;
                    let build = WarmBuild {
                        key,
                        snapshot: cold,
                        command: warm.command.clone(),
                        network: warm.network,
                        timeout_seconds: warm.timeout_seconds,
                    };
                    (None, Some(build))
                }
            }
        };
        self.dispatch(RecordPrepared {
            run: run.id(),
            plan,
            snapshot,
            warm,
        })
        .await
    }

    async fn start_warm(&self, run: &Run, build: &WarmBuild) -> Result<(), AppError> {
        let settings = Self::settings(run)?;
        let timeout = Self::timeout(build.timeout_seconds)?;
        let spec = BuildSpec {
            repo: run.repo(),
            key: build.key,
            commit: run.commit().clone(),
            sandbox: Self::sandbox_spec(run, build.snapshot, settings, build.network),
            command: build.command.clone(),
            secrets: settings.secrets.clone(),
            timeout,
        };
        let build = self.dispatch(StartBuild { spec }).await?;
        self.dispatch(RecordWarmBuilding {
            run: run.id(),
            build,
        })
        .await
    }

    /// Checks the run's commit out over the warm snapshot.
    async fn check_out(&self, run: &Run, warm: &WarmSnapshot) -> Result<(), AppError> {
        let snapshot = self
            .checkouts
            .checkout(run.repo(), run.commit(), None, Some(warm))
            .await?;
        self.dispatch(RecordCheckedOut {
            run: run.id(),
            snapshot,
        })
        .await
    }

    async fn start_checks(&self, run: &Run, snapshot: SnapshotId) -> Result<(), AppError> {
        let settings = Self::settings(run)?;
        let sandbox = self
            .sandbox(run, snapshot, settings, settings.network)
            .await?;
        let mut jobs = Vec::with_capacity(run.checks().len());
        for check in run.checks() {
            jobs.push(
                self.job(
                    sandbox,
                    &check.spec.run,
                    &settings.secrets,
                    check.spec.timeout_seconds,
                )
                .await?,
            );
        }
        self.dispatch(RecordChecksStarted {
            run: run.id(),
            sandbox,
            jobs,
        })
        .await
    }

    async fn sandbox(
        &self,
        run: &Run,
        snapshot: SnapshotId,
        settings: &SandboxSettings,
        network: NetworkPolicy,
    ) -> Result<SandboxId, AppError> {
        let spec = Self::sandbox_spec(run, snapshot, settings, network);
        self.dispatch(CreateSandbox { spec }).await
    }

    fn sandbox_spec(
        run: &Run,
        snapshot: SnapshotId,
        settings: &SandboxSettings,
        network: NetworkPolicy,
    ) -> SandboxSpec {
        SandboxSpec::builder()
            .snapshot(snapshot)
            .repo(run.repo())
            .isolation(settings.isolation)
            .limits(settings.limits)
            .network(network)
            .env(settings.env.clone())
            .build()
    }

    async fn job(
        &self,
        sandbox: SandboxId,
        command: &str,
        secrets: &BTreeSet<SecretName>,
        timeout_seconds: u32,
    ) -> Result<JobId, AppError> {
        let argv = Argv::try_from(vec!["sh".to_owned(), "-c".to_owned(), command.to_owned()])
            .map_err(|error| {
                AppError::Validation(igloo_core::ValidationErrors::single(
                    "run",
                    error.to_string(),
                ))
            })?;
        let timeout = Self::timeout(timeout_seconds)?;
        let spec = JobSpec::Execute {
            sandbox,
            argv,
            env: EnvVars::default(),
            secrets: secrets.clone(),
            timeout,
        };
        self.dispatch(SubmitJob { spec }).await
    }

    fn timeout(seconds: u32) -> Result<JobTimeout, AppError> {
        JobTimeout::try_from(SignedDuration::from_secs(i64::from(seconds))).map_err(|error| {
            AppError::Validation(igloo_core::ValidationErrors::single(
                "timeout_seconds",
                error.to_string(),
            ))
        })
    }

    fn settings(run: &Run) -> Result<&SandboxSettings, AppError> {
        run.settings()
            .ok_or_else(|| AppError::not_found("run_plan", &run.id()))
    }

    async fn repo(&self, run: &Run) -> Result<Repo, AppError> {
        self.repos
            .get(run.repo())
            .await?
            .ok_or_else(|| AppError::not_found(Repo::NAME, &run.repo()))
    }

    async fn fail(&self, run: &Run, reason: String) -> Result<(), AppError> {
        self.dispatch(FailRun {
            run: run.id(),
            reason,
        })
        .await
    }

    async fn dispatch<C: crate::app::Command>(&self, command: C) -> Result<C::Output, AppError> {
        let context = RequestContext::new(
            Actor::System {
                component: SystemComponent::Controller,
            },
            self.ids.next(),
        );
        self.bus.dispatch(command, context).await
    }
}
