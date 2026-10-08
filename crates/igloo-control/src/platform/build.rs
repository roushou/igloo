use std::sync::Arc;

use async_trait::async_trait;
use igloo_core::build::{Build, BuildAction, BuildError, BuildId, BuildOutcome, BuildSpec};
use igloo_core::job::{Job, JobEnding, JobEvent, JobId, JobSpec};
use igloo_core::process::{Argv, EnvVars};
use igloo_core::repo::{RepoId, WarmSnapshot};
use igloo_core::seal::{Seal, SealEvent, SealFailure, SealId};
use igloo_core::snapshot::SnapshotId;
use igloo_core::{Actor, Digest, Entity, SystemComponent, ValidationErrors};

use super::{CreateSandbox, CreateSeal, RecordWarmSnapshot, StopSandbox, SubmitJob};
use crate::app::{
    AppError, Command, CommandBus, CommandHandler, ControllerSettings, EntityHandler, Extension,
    InstallError, PlatformBuilder, Reactor, Reconciler, RequestContext,
};
use crate::ports::{
    Clock, EntityStore, EventEnvelope, IdGenerator, IdGeneratorExt, StorageError, Versioned,
};

/// Builds a snapshot, or returns the build of the same repository and key that is in progress
/// or built: the same key builds the same snapshot.
pub struct StartBuild {
    /// What to build.
    pub spec: BuildSpec,
}

/// Records a build's sandbox and the job running its command.
pub struct RecordBuildStarted {
    /// The build.
    pub build: BuildId,
    /// Its sandbox.
    pub sandbox: igloo_core::sandbox::SandboxId,
    /// Its command's job.
    pub job: JobId,
}

/// Records how a job ended, for the build it runs in.
pub struct RecordBuildJob {
    /// The build.
    pub build: BuildId,
    /// The job.
    pub job: JobId,
    /// How it ended.
    pub ending: JobEnding,
}

/// Records a build's seal.
pub struct RecordBuildSealing {
    /// The build.
    pub build: BuildId,
    /// The seal.
    pub seal: SealId,
}

/// Records how a build's seal ended.
pub struct RecordBuildSeal {
    /// The build.
    pub build: BuildId,
    /// The seal.
    pub seal: SealId,
    /// The snapshot, or why the seal failed.
    pub snapshot: Result<SnapshotId, String>,
}

/// Records that a build's snapshot is on its repository.
pub struct RecordBuildRecorded {
    /// The build.
    pub build: BuildId,
}

/// Records that a build's sandbox was stopped.
pub struct RecordBuildSandboxStopped {
    /// The build.
    pub build: BuildId,
}

impl Command for StartBuild {
    type Output = BuildId;
    const NAME: &'static str = "build.start";
}

impl Command for RecordBuildStarted {
    type Output = ();
    const NAME: &'static str = "build.record_started";
}

impl Command for RecordBuildJob {
    type Output = ();
    const NAME: &'static str = "build.record_job";
}

impl Command for RecordBuildSealing {
    type Output = ();
    const NAME: &'static str = "build.record_sealing";
}

impl Command for RecordBuildSeal {
    type Output = ();
    const NAME: &'static str = "build.record_seal";
}

impl Command for RecordBuildRecorded {
    type Output = ();
    const NAME: &'static str = "build.record_recorded";
}

impl Command for RecordBuildSandboxStopped {
    type Output = ();
    const NAME: &'static str = "build.record_sandbox_stopped";
}

/// Read access to builds beyond loading one by id.
#[derive(Clone)]
pub struct BuildQueries {
    store: Arc<dyn EntityStore<Build>>,
}

impl BuildQueries {
    /// Queries over `store`.
    #[must_use]
    pub fn new(store: Arc<dyn EntityStore<Build>>) -> Self {
        Self { store }
    }

    /// The build `id`.
    pub async fn get(&self, id: BuildId) -> Result<Option<Build>, StorageError> {
        Ok(self.store.load(id).await?.map(Versioned::into_inner))
    }

    /// The build whose command runs as `job`.
    pub async fn with_job(&self, job: JobId) -> Result<Option<Build>, StorageError> {
        self.find(|build| build.job() == Some(job)).await
    }

    /// The build being sealed by `seal`.
    pub async fn with_seal(&self, seal: SealId) -> Result<Option<Build>, StorageError> {
        self.find(|build| build.seal() == Some(seal)).await
    }

    /// The build of `repo` under `key` that is in progress or built, if any.
    pub async fn reusable(
        &self,
        repo: RepoId,
        key: &Digest,
    ) -> Result<Option<Build>, StorageError> {
        self.find(|build| {
            !matches!(build.outcome(), Some(BuildOutcome::Failed { .. }))
                && build.spec().repo == repo
                && build.spec().key == *key
        })
        .await
    }

    async fn find(&self, matches: impl Fn(&Build) -> bool) -> Result<Option<Build>, StorageError> {
        Ok(self
            .store
            .all()
            .await?
            .into_iter()
            .find(|build| matches(build)))
    }
}

/// Starts a build unless one of the same repository and key is in progress or built.
struct StartBuildHandler {
    queries: BuildQueries,
    store: Arc<dyn EntityStore<Build>>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
}

#[async_trait]
impl CommandHandler<StartBuild> for StartBuildHandler {
    async fn handle(
        &self,
        command: StartBuild,
        context: &RequestContext,
    ) -> Result<BuildId, AppError> {
        let spec = command.spec;
        if let Some(build) = self.queries.reusable(spec.repo, &spec.key).await? {
            return Ok(build.id());
        }
        let id = self.ids.next::<Build>();
        let now = self.clock.now();
        self.store
            .commit(
                &mut Versioned::new(Build::new(id, spec, now)),
                &context.commit_meta(now),
            )
            .await?;
        Ok(id)
    }
}

/// Carries out builds' plans: starts the command, seals the sandbox, records the snapshot on
/// the repository and stops the sandbox.
struct Builder {
    bus: CommandBus,
    ids: Arc<dyn IdGenerator>,
}

impl Reconciler<Build> for Builder {
    async fn reconcile(&self, build: &Build, actions: Vec<BuildAction>) -> Result<(), AppError> {
        let id = build.id();
        for action in actions {
            match action {
                BuildAction::Start => self.start(build).await?,
                BuildAction::Seal(sandbox) => {
                    let seal = self.dispatch(CreateSeal { sandbox }).await?;
                    self.dispatch(RecordBuildSealing { build: id, seal })
                        .await?;
                }
                BuildAction::Record(snapshot) => {
                    let spec = build.spec();
                    self.dispatch(RecordWarmSnapshot {
                        repo: spec.repo,
                        key: spec.key,
                        warm: WarmSnapshot {
                            snapshot,
                            commit: spec.commit.clone(),
                        },
                    })
                    .await?;
                    self.dispatch(RecordBuildRecorded { build: id }).await?;
                }
                BuildAction::StopSandbox(sandbox) => {
                    self.dispatch(StopSandbox { sandbox }).await?;
                    self.dispatch(RecordBuildSandboxStopped { build: id })
                        .await?;
                }
            }
        }
        Ok(())
    }
}

impl Builder {
    async fn start(&self, build: &Build) -> Result<(), AppError> {
        let spec = build.spec();
        let sandbox = self
            .dispatch(CreateSandbox {
                spec: spec.sandbox.clone(),
            })
            .await?;
        let argv = Argv::try_from(vec!["sh".to_owned(), "-c".to_owned(), spec.command.clone()])
            .map_err(|error| {
                AppError::Validation(ValidationErrors::single("command", error.to_string()))
            })?;
        let job = self
            .dispatch(SubmitJob {
                spec: JobSpec::Execute {
                    sandbox,
                    argv,
                    env: EnvVars::default(),
                    secrets: spec.secrets.clone(),
                    timeout: spec.timeout,
                },
            })
            .await?;
        self.dispatch(RecordBuildStarted {
            build: build.id(),
            sandbox,
            job,
        })
        .await
    }

    async fn dispatch<C: Command>(&self, command: C) -> Result<C::Output, AppError> {
        let context = RequestContext::new(
            Actor::System {
                component: SystemComponent::Controller,
            },
            self.ids.next(),
        );
        self.bus.dispatch(command, context).await
    }
}

/// Records how the jobs builds wait on ended.
struct RecordBuildJobs {
    builds: BuildQueries,
}

/// Records how the seals builds wait on ended.
struct RecordBuildSeals {
    builds: BuildQueries,
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
impl Reactor for RecordBuildJobs {
    fn name(&self) -> &'static str {
        "build.record_jobs"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let Some(job) = event.subject_as::<Job>() else {
            return Ok(());
        };
        let Some(ending) = event.decode::<JobEvent>()?.ending() else {
            return Ok(());
        };
        let Some(build) = self.builds.with_job(job).await? else {
            return Ok(());
        };
        let command = RecordBuildJob {
            build: build.id(),
            job,
            ending,
        };
        bus.dispatch(command, context(event)).await
    }
}

#[async_trait]
impl Reactor for RecordBuildSeals {
    fn name(&self) -> &'static str {
        "build.record_seals"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let Some(seal) = event.subject_as::<Seal>() else {
            return Ok(());
        };
        let snapshot = match event.decode::<SealEvent>()? {
            SealEvent::Sealed { snapshot } => Ok(snapshot),
            SealEvent::Failed { reason } => Err(match reason {
                SealFailure::SandboxEnded => "the sandbox ended".to_owned(),
                SealFailure::WorkerError => "the worker could not seal".to_owned(),
            }),
            SealEvent::Requested { .. } => return Ok(()),
        };
        let Some(build) = self.builds.with_seal(seal).await? else {
            return Ok(());
        };
        let command = RecordBuildSeal {
            build: build.id(),
            seal,
            snapshot,
        };
        bus.dispatch(command, context(event)).await
    }
}

/// Builds: their commands, controller, and the reactors feeding it job and seal outcomes.
pub struct BuildModule {
    /// Pacing of the build controller.
    pub settings: ControllerSettings,
}

impl Extension for BuildModule {
    fn name(&self) -> &'static str {
        "builds"
    }

    fn install(self: Box<Self>, platform: &mut PlatformBuilder) -> Result<(), InstallError> {
        let ports = platform.ports().clone();
        let store = platform.store::<Build>()?;
        let queries = BuildQueries::new(Arc::clone(&store));
        let clock = &ports.clock;
        platform.command(StartBuildHandler {
            queries: queries.clone(),
            store: Arc::clone(&store),
            clock: Arc::clone(clock),
            ids: Arc::clone(&ports.ids),
        })?;
        platform.command(EntityHandler::new(
            Arc::clone(&store),
            Arc::clone(clock),
            |command: &RecordBuildStarted| command.build,
            |build: &mut Build, command: &RecordBuildStarted, _| -> Result<(), BuildError> {
                build.started(command.sandbox, command.job)
            },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(&store),
            Arc::clone(clock),
            |command: &RecordBuildJob| command.build,
            |build: &mut Build, command: &RecordBuildJob, _| {
                build.job_ended(command.job, command.ending);
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(&store),
            Arc::clone(clock),
            |command: &RecordBuildSealing| command.build,
            |build: &mut Build, command: &RecordBuildSealing, _| -> Result<(), BuildError> {
                build.sealing(command.seal)
            },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(&store),
            Arc::clone(clock),
            |command: &RecordBuildSeal| command.build,
            |build: &mut Build, command: &RecordBuildSeal, _| {
                build.seal_ended(command.seal, command.snapshot.clone());
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(&store),
            Arc::clone(clock),
            |command: &RecordBuildRecorded| command.build,
            |build: &mut Build, _: &RecordBuildRecorded, _| -> Result<(), BuildError> {
                build.recorded()
            },
        ))?;
        platform.command(EntityHandler::infallible(
            store,
            Arc::clone(clock),
            |command: &RecordBuildSandboxStopped| command.build,
            |build: &mut Build, _: &RecordBuildSandboxStopped, _| build.sandbox_stopped(),
        ))?;
        platform.controller(
            Builder {
                bus: platform.bus(),
                ids: Arc::clone(&ports.ids),
            },
            self.settings,
        )?;
        platform.reactor(RecordBuildJobs {
            builds: queries.clone(),
        });
        platform.reactor(RecordBuildSeals { builds: queries });
        Ok(())
    }
}
