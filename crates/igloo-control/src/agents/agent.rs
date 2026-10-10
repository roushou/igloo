use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use igloo_core::build::BuildSpec;
use igloo_core::change::Change;
use igloo_core::job::{JobId, JobSpec, JobTimeout};
use igloo_core::process::{Argv, EnvVars, OutputStream};
use igloo_core::repo::{BranchName, CommitId, Repo, RepoId, SecretName, WarmSnapshot};
use igloo_core::sandbox::{NetworkPolicy, SandboxSpec};
use igloo_core::snapshot::SnapshotId;
use igloo_core::{Actor, Digest, Entity, SystemComponent, ValidationErrors};
use jiff::SignedDuration;

use super::AgentSettings;
use super::bundle::{CommitBundle, GitIdentity};
use super::commands::{
    FailTask, RecordCommitsCollecting, RecordTaskBuilding, RecordTaskPrepared, RecordTaskReady,
    RecordTaskRevised, RecordTaskSandbox, RecordTaskSandboxStopped, RecordTurnStarted,
};
use super::task::{Prepared, Task, TaskAction};
use crate::app::{AppError, Command, CommandBus, Reconciler, RequestContext};
use crate::ci::{Environment, Environments};
use crate::platform::{
    ChangeHeads, ChangeQueries, CreateSandbox, OpenChange, RecordWarmUse, RepoQueries,
    RepoSnapshots, ReviseChange, StartBuild, StopSandbox, SubmitJob, WarmRecipe,
};
use crate::ports::{Expected, Forge, IdGenerator, IdGeneratorExt, LogStore};

/// Carries out tasks' plans: reads their settings, builds and checks out their snapshot,
/// starts their sandbox and turns, and stops the sandbox once they end.
pub(super) struct Agent {
    pub(super) bus: CommandBus,
    pub(super) ids: Arc<dyn IdGenerator>,
    pub(super) repos: RepoQueries,
    pub(super) forge: Arc<dyn Forge>,
    pub(super) checkouts: RepoSnapshots,
    pub(super) environments: Environments,
    pub(super) logs: Arc<dyn LogStore>,
    pub(super) heads: ChangeHeads,
    pub(super) changes: ChangeQueries,
}

/// What a step of the setup decided.
enum Step {
    /// The task's sandbox starts from this snapshot.
    Ready(SnapshotId),
    /// This build comes first.
    Build(Box<BuildSpec>),
}

impl Reconciler<Task> for Agent {
    async fn reconcile(&self, task: &Task, actions: Vec<TaskAction>) -> Result<(), AppError> {
        for action in actions {
            match action {
                TaskAction::Prepare => self.prepare(task).await?,
                TaskAction::StartSandbox(snapshot) => self.start_sandbox(task, snapshot).await?,
                TaskAction::StartTurn(prompt) => self.start_turn(task, prompt).await?,
                TaskAction::CollectCommits => self.collect(task).await?,
                TaskAction::Publish(job) => self.publish(task, job).await?,
                TaskAction::StopSandbox(sandbox) => {
                    self.dispatch(StopSandbox { sandbox }).await?;
                    self.dispatch(RecordTaskSandboxStopped { task: task.id() })
                        .await?;
                }
            }
        }
        Ok(())
    }
}

impl Agent {
    /// How long collecting a turn's commits may take, in seconds.
    const COLLECT_TIMEOUT: u32 = 600;
    const PAGE: usize = 256;

    /// Settles the task's settings once, then finds its snapshot or starts the next build.
    async fn prepare(&self, task: &Task) -> Result<(), AppError> {
        let repo = self.repo(task).await?;
        let prepared = match task.settings() {
            Some(prepared) => prepared.clone(),
            None => match self.settle(task, &repo).await? {
                Ok(prepared) => {
                    self.dispatch(RecordTaskPrepared {
                        task: task.id(),
                        prepared: prepared.clone(),
                    })
                    .await?;
                    prepared
                }
                Err(reason) => return self.fail(task, reason).await,
            },
        };
        match self.step(&repo, &prepared).await? {
            Step::Ready(snapshot) => {
                self.dispatch(RecordTaskReady {
                    task: task.id(),
                    snapshot,
                })
                .await
            }
            Step::Build(spec) => {
                let build = self.dispatch(StartBuild { spec: *spec }).await?;
                self.dispatch(RecordTaskBuilding {
                    task: task.id(),
                    build,
                })
                .await
            }
        }
    }

    /// Reads the tool and the pipeline at the default branch's head, or why the task cannot
    /// run.
    async fn settle(&self, task: &Task, repo: &Repo) -> Result<Result<Prepared, String>, AppError> {
        let remote = self.repos.remote(repo).await?;
        let commit = self.forge.fetch(&remote, repo.default_branch()).await?;
        let text = self
            .forge
            .read_file(repo.id(), &commit, AgentSettings::PATH)
            .await?
            .unwrap_or_default();
        let settings = match std::str::from_utf8(&text)
            .map_err(|error| error.to_string())
            .and_then(|text| AgentSettings::try_from(text).map_err(|errors| errors.to_string()))
        {
            Ok(settings) => settings,
            Err(reason) => return Ok(Err(format!("invalid agent settings: {reason}"))),
        };
        let Some((tool, spec)) = settings.tool(task.tool()) else {
            return Ok(Err(match task.tool() {
                Some(tool) => format!("the repository has no tool {tool}"),
                None => "the repository sets no default tool".to_owned(),
            }));
        };
        let Environment { pipeline, base } = match self.environments.at(repo, &commit).await? {
            Ok(environment) => environment,
            Err(reason) => return Ok(Err(reason)),
        };
        Ok(Ok(Prepared {
            commit,
            tool: tool.clone(),
            spec: spec.clone(),
            settings: pipeline.sandbox,
            base,
            warm: pipeline.warm,
        }))
    }

    /// The next step toward the task's snapshot: the warm snapshot first, then the tool's
    /// agent snapshot over it, then the checkout of the task's commit over that.
    async fn step(&self, repo: &Repo, prepared: &Prepared) -> Result<Step, AppError> {
        let commit = &prepared.commit;
        let mut used = Vec::new();
        let (key, over) = if let Some(warm) = &prepared.warm {
            let recipe = WarmRecipe {
                command: warm.command.clone(),
                lockfiles: warm.lockfiles.clone(),
            };
            let key = self
                .checkouts
                .warm_key(repo.id(), commit, prepared.base, &recipe)
                .await?;
            let Some(built) = repo.warm(&key) else {
                let recipe = Recipe {
                    snapshot: self.cold(repo, prepared).await?,
                    commit: commit.clone(),
                    key,
                    command: warm.command.clone(),
                    network: warm.network,
                    secrets: prepared.settings.secrets.clone(),
                    timeout_seconds: warm.timeout_seconds,
                };
                return Self::build_step(repo, prepared, recipe);
            };
            used.push(key);
            (key, built.clone())
        } else {
            let cold = self.cold(repo, prepared).await?;
            if prepared.spec.install.is_none() {
                return Ok(Step::Ready(cold));
            }
            // Without a warm snapshot, the agent snapshot is keyed by what the checkout is of:
            // the base and the commit.
            let base = prepared
                .base
                .map_or_else(|| "-".to_owned(), |base| base.to_string());
            let key = Digest::from_blake3(
                *blake3::hash(format!("igloo.cold.v1\nbase {base}\ncommit {commit}\n").as_bytes())
                    .as_bytes(),
            );
            let over = WarmSnapshot {
                snapshot: cold,
                commit: commit.clone(),
            };
            (key, over)
        };
        let agent = if let Some(install) = &prepared.spec.install {
            let agent_key = prepared.spec.snapshot_key(&key);
            let Some(built) = repo.warm(&agent_key) else {
                let recipe = Recipe {
                    snapshot: over.snapshot,
                    commit: over.commit,
                    key: agent_key,
                    command: install.clone(),
                    network: NetworkPolicy::AllowAll,
                    secrets: BTreeSet::new(),
                    timeout_seconds: prepared.spec.timeout_seconds,
                };
                return Self::build_step(repo, prepared, recipe);
            };
            used.push(agent_key);
            built.clone()
        } else {
            over
        };
        if !used.is_empty() {
            self.dispatch(RecordWarmUse {
                repo: repo.id(),
                keys: used,
            })
            .await?;
        }
        let snapshot = self
            .checkouts
            .checkout(repo.id(), commit, None, Some(&agent))
            .await?;
        Ok(Step::Ready(snapshot))
    }

    /// The checkout of the task's commit over its base.
    async fn cold(&self, repo: &Repo, prepared: &Prepared) -> Result<SnapshotId, AppError> {
        self.checkouts
            .checkout(repo.id(), &prepared.commit, prepared.base, None)
            .await
    }

    fn build_step(repo: &Repo, prepared: &Prepared, recipe: Recipe) -> Result<Step, AppError> {
        Ok(Step::Build(Box::new(BuildSpec {
            repo: repo.id(),
            key: recipe.key,
            commit: recipe.commit,
            sandbox: Self::sandbox_spec(repo.id(), prepared, recipe.snapshot, recipe.network),
            command: recipe.command,
            secrets: recipe.secrets,
            timeout: Self::timeout(recipe.timeout_seconds)?,
        })))
    }

    async fn start_sandbox(&self, task: &Task, snapshot: SnapshotId) -> Result<(), AppError> {
        let prepared = Self::prepared(task)?;
        let spec = Self::sandbox_spec(task.repo(), prepared, snapshot, NetworkPolicy::AllowAll);
        let sandbox = self.dispatch(CreateSandbox { spec }).await?;
        self.dispatch(RecordTaskSandbox {
            task: task.id(),
            sandbox,
        })
        .await
    }

    async fn start_turn(&self, task: &Task, prompt: String) -> Result<(), AppError> {
        let prepared = Self::prepared(task)?;
        let Some(sandbox) = task.sandbox() else {
            return Err(AppError::not_found("task_sandbox", &task.id()));
        };
        let turn = prepared
            .spec
            .harness
            .harness()
            .turn(&prepared.spec, &prompt, !task.turns().is_empty())
            .map_err(AppError::Validation)?;
        let env = BTreeMap::from(turn.env)
            .into_iter()
            .chain(GitIdentity::of(&prepared.tool).env())
            .collect::<BTreeMap<_, _>>();
        let job = self
            .dispatch(SubmitJob {
                spec: JobSpec::Execute {
                    sandbox,
                    argv: turn.argv,
                    env: EnvVars::try_from(env).map_err(AppError::Validation)?,
                    secrets: prepared.spec.secrets.clone(),
                    timeout: Self::timeout(prepared.spec.timeout_seconds)?,
                },
            })
            .await?;
        self.dispatch(RecordTurnStarted {
            task: task.id(),
            job,
            prompt,
        })
        .await
    }

    /// Submits the job writing the last turn's commits to its output as a bundle.
    async fn collect(&self, task: &Task) -> Result<(), AppError> {
        let prepared = Self::prepared(task)?;
        let Some(sandbox) = task.sandbox() else {
            return Err(AppError::not_found("task_sandbox", &task.id()));
        };
        let argv = Argv::try_from(vec![
            "sh".to_owned(),
            "-c".to_owned(),
            CommitBundle::SCRIPT.to_owned(),
        ])
        .map_err(|error| {
            AppError::Validation(ValidationErrors::single("argv", error.to_string()))
        })?;
        let env = GitIdentity::of(&prepared.tool)
            .env()
            .into_iter()
            .chain([
                (CommitBundle::BASE.to_owned(), prepared.commit.to_string()),
                (CommitBundle::MESSAGE.to_owned(), task.title()),
                (CommitBundle::TASK.to_owned(), task.id().to_string()),
            ])
            .collect::<BTreeMap<_, _>>();
        let job = self
            .dispatch(SubmitJob {
                spec: JobSpec::Execute {
                    sandbox,
                    argv,
                    env: EnvVars::try_from(env).map_err(AppError::Validation)?,
                    secrets: BTreeSet::new(),
                    timeout: Self::timeout(Self::COLLECT_TIMEOUT)?,
                },
            })
            .await?;
        self.dispatch(RecordCommitsCollecting {
            task: task.id(),
            job,
        })
        .await
    }

    /// Advances the task's branch in Igloo's copy to the commits `job` collected and records them as the next
    /// revision of the task's change, opening it on the first. A turn without commits fails
    /// the task.
    async fn publish(&self, task: &Task, job: JobId) -> Result<(), AppError> {
        let number = task.turns().len();
        let output = self.stdout(job).await?;
        let bundle = match CommitBundle::decode(&output) {
            Ok(Some(bundle)) => bundle,
            Ok(None) => {
                return self
                    .fail(task, format!("turn {number} made no commits"))
                    .await;
            }
            Err(error) => return self.fail(task, format!("turn {number}: {error}")).await,
        };
        let repo = self.repo(task).await?;
        let scratch = tempfile::tempdir().map_err(AppError::infrastructure)?;
        let path = scratch.path().join("task.bundle");
        tokio::fs::write(&path, &bundle)
            .await
            .map_err(AppError::infrastructure)?;
        let head = self.forge.import(repo.id(), &path).await?;
        let branch: BranchName = format!("igloo/tasks/{}", task.id()).parse().map_err(
            |error: igloo_core::repo::RepoValueError| {
                AppError::Validation(ValidationErrors::single("branch", error.to_string()))
            },
        )?;
        self.forge
            .advance(repo.id(), &branch, &head, Expected::Any)
            .await?;
        let heads = self.heads.held(&repo, &branch).await?;
        let change = match task.change() {
            Some(change) => {
                self.dispatch(ReviseChange { change, heads }).await?;
                change
            }
            None => {
                self.dispatch(OpenChange {
                    repo: repo.id(),
                    source: branch,
                    title: task.title(),
                    heads,
                })
                .await?
            }
        };
        let revision = self
            .changes
            .get(change)
            .await?
            .and_then(|change| change.revisions().last().map(|revision| revision.number))
            .ok_or_else(|| AppError::not_found(Change::NAME, &change))?;
        self.dispatch(RecordTaskRevised {
            task: task.id(),
            change,
            revision,
        })
        .await
    }

    /// Everything `job` wrote to its standard output.
    async fn stdout(&self, job: JobId) -> Result<Vec<u8>, AppError> {
        let mut output = Vec::new();
        let mut after = 0;
        loop {
            let page = self.logs.read(job, after, Self::PAGE).await?;
            let Some(last) = page.last() else {
                return Ok(output);
            };
            after = last.sequence;
            for entry in &page {
                if entry.stream == OutputStream::Stdout {
                    output.extend_from_slice(&entry.data);
                }
            }
        }
    }

    fn sandbox_spec(
        repo: RepoId,
        prepared: &Prepared,
        snapshot: SnapshotId,
        network: NetworkPolicy,
    ) -> SandboxSpec {
        let settings = &prepared.settings;
        SandboxSpec::builder()
            .snapshot(snapshot)
            .repo(repo)
            .isolation(settings.isolation)
            .limits(settings.limits)
            .network(network)
            .env(settings.env.clone())
            .build()
    }

    fn timeout(seconds: u32) -> Result<JobTimeout, AppError> {
        JobTimeout::try_from(SignedDuration::from_secs(i64::from(seconds))).map_err(|error| {
            AppError::Validation(ValidationErrors::single(
                "timeout_seconds",
                error.to_string(),
            ))
        })
    }

    fn prepared(task: &Task) -> Result<&Prepared, AppError> {
        task.settings()
            .ok_or_else(|| AppError::not_found("task_settings", &task.id()))
    }

    async fn repo(&self, task: &Task) -> Result<Repo, AppError> {
        self.repos
            .get(task.repo())
            .await?
            .ok_or_else(|| AppError::not_found(Repo::NAME, &task.repo()))
    }

    async fn fail(&self, task: &Task, reason: String) -> Result<(), AppError> {
        self.dispatch(FailTask {
            task: task.id(),
            reason,
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

/// A snapshot the task needs: `command` run over `snapshot`, recorded under `key`.
struct Recipe {
    snapshot: SnapshotId,
    commit: CommitId,
    key: Digest,
    command: String,
    network: NetworkPolicy,
    secrets: BTreeSet<SecretName>,
    timeout_seconds: u32,
}
