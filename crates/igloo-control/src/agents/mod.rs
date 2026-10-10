//! Agents: coding tools working on a repository's tasks in sandboxes.

mod agent;
mod bundle;
mod commands;
mod handback;
mod harness;
mod reactors;
mod settings;
mod task;
mod transcript;

use std::sync::Arc;

use igloo_core::build::Build;
use igloo_core::change::Change;
use igloo_core::repo::Repo;

use self::agent::Agent;
pub use self::bundle::{CommitBundle, InvalidBundle};
pub use self::commands::{
    CancelTask, CreateTask, FailTask, FinishTask, HandBackTask, RecordChangesReading,
    RecordCommitsCollecting, RecordHandedBack, RecordTaskBuilding, RecordTaskBuilt,
    RecordTaskPrepared, RecordTaskReady, RecordTaskRevised, RecordTaskSandbox,
    RecordTaskSandboxStopped, RecordTurnEnded, RecordTurnStarted, RequestTurn, TakeOverTask,
    TaskQueries,
};
use self::commands::{CreateTaskHandler, TakeoverHandler};
pub use self::handback::{InvalidChanges, PersonChanges};
pub use self::harness::{ClaudeCode, Codex, CommandHarness, Harness, TurnCommand};
use self::reactors::{AttributeOutcomes, FollowReviews, RecordTaskBuilds, RecordTurns};
pub use self::settings::{AgentSettings, HarnessKind, InvalidToolName, ToolName, ToolSpec};
pub use self::task::{
    Prepared, TakeoverPhase, Task, TaskAction, TaskEnd, TaskError, TaskEvent, TaskId, TaskPhase,
    Turn,
};
pub use self::transcript::{Entry, TranscriptEntry, Transcripts};
use crate::app::{ControllerSettings, EntityHandler, Extension, InstallError, PlatformBuilder};
use crate::ci::CiModule;
use crate::platform::{BuildQueries, ChangeHeads, ChangeQueries, RepoQueries, RepoSnapshots};
use crate::ports::EntityStore;

/// Agents: tasks, their controller, and the reactors feeding it turn and build outcomes.
pub struct AgentsModule {
    /// Pacing of the agent controller.
    pub settings: ControllerSettings,
}

impl Extension for AgentsModule {
    fn name(&self) -> &'static str {
        "agents"
    }

    fn install(self: Box<Self>, platform: &mut PlatformBuilder) -> Result<(), InstallError> {
        let store = platform.store::<Task>()?;
        let tasks = TaskQueries::new(Arc::clone(&store));
        Self::commands(platform, &store)?;
        Self::turn_commands(platform, &store)?;
        let ports = platform.ports().clone();
        let agent = Agent {
            bus: platform.bus(),
            ids: Arc::clone(&ports.ids),
            repos: RepoQueries::new(platform.store::<Repo>()?, Arc::clone(&ports.secrets)),
            forge: Arc::clone(&ports.forge),
            checkouts: RepoSnapshots::new(Arc::clone(&ports.forge), Arc::clone(&ports.blobs)),
            environments: CiModule::environments(platform),
            logs: Arc::clone(&ports.logs),
            heads: ChangeHeads::new(
                RepoQueries::new(platform.store::<Repo>()?, Arc::clone(&ports.secrets)),
                Arc::clone(&ports.forge),
            ),
            changes: ChangeQueries::new(platform.store::<Change>()?),
            tasks: tasks.clone(),
        };
        platform.controller(agent, self.settings)?;
        platform.reactor(RecordTurns {
            tasks: tasks.clone(),
            logs: Arc::clone(&ports.logs),
        });
        platform.reactor(FollowReviews {
            tasks: tasks.clone(),
        });
        platform.reactor(AttributeOutcomes {
            tasks: tasks.clone(),
        });
        platform.reactor(RecordTaskBuilds {
            tasks,
            builds: BuildQueries::new(platform.store::<Build>()?),
        });
        Ok(())
    }
}

impl AgentsModule {
    /// The commands creating tasks and recording how their snapshot and sandbox are set up.
    fn commands(
        platform: &mut PlatformBuilder,
        store: &Arc<dyn EntityStore<Task>>,
    ) -> Result<(), InstallError> {
        let ports = platform.ports().clone();
        let clock = &ports.clock;
        platform.command(CreateTaskHandler {
            store: Arc::clone(store),
            repos: platform.store::<Repo>()?,
            clock: Arc::clone(clock),
            ids: Arc::clone(&ports.ids),
        })?;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordTaskPrepared| command.task,
            |task: &mut Task, command: &RecordTaskPrepared, _| -> Result<(), TaskError> {
                task.prepared(command.prepared.clone())
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordTaskBuilding| command.task,
            |task: &mut Task, command: &RecordTaskBuilding, _| -> Result<(), TaskError> {
                task.building(command.build)
            },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordTaskBuilt| command.task,
            |task: &mut Task, command: &RecordTaskBuilt, _| {
                task.built(command.build, command.result.clone());
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordTaskReady| command.task,
            |task: &mut Task, command: &RecordTaskReady, _| -> Result<(), TaskError> {
                task.ready(command.snapshot)
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordTaskSandbox| command.task,
            |task: &mut Task, command: &RecordTaskSandbox, _| -> Result<(), TaskError> {
                task.sandbox_started(command.sandbox)
            },
        ))?;
        Ok(())
    }

    /// The commands recording turns and ending tasks.
    fn turn_commands(
        platform: &mut PlatformBuilder,
        store: &Arc<dyn EntityStore<Task>>,
    ) -> Result<(), InstallError> {
        let ports = platform.ports().clone();
        let clock = &ports.clock;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordTurnStarted| command.task,
            |task: &mut Task, command: &RecordTurnStarted, _| -> Result<(), TaskError> {
                task.turn_started(command.job, command.prompt.clone())
            },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordTurnEnded| command.task,
            |task: &mut Task, command: &RecordTurnEnded, now| {
                if command.limited {
                    task.turn_limited(command.job, command.ending, now);
                } else {
                    task.job_ended(command.job, command.ending);
                }
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordCommitsCollecting| command.task,
            |task: &mut Task, command: &RecordCommitsCollecting, _| -> Result<(), TaskError> {
                task.collecting(command.job)
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordTaskRevised| command.task,
            |task: &mut Task, command: &RecordTaskRevised, _| -> Result<(), TaskError> {
                task.revised(command.change, command.revision)
            },
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &FailTask| command.task,
            |task: &mut Task, command: &FailTask, _| task.fail(command.reason.clone()),
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RequestTurn| command.task,
            |task: &mut Task, command: &RequestTurn, _| task.request_turn(command.prompt.clone()),
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &FinishTask| command.task,
            |task: &mut Task, _: &FinishTask, _| task.finish(),
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &CancelTask| command.task,
            |task: &mut Task, _: &CancelTask, _| task.cancel(),
        ))?;
        platform.command(EntityHandler::infallible(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordTaskSandboxStopped| command.task,
            |task: &mut Task, _: &RecordTaskSandboxStopped, _| task.sandbox_stopped(),
        ))?;
        let takeover = TakeoverHandler {
            store: Arc::clone(store),
            clock: Arc::clone(clock),
        };
        platform.command::<TakeOverTask>(takeover.clone())?;
        platform.command::<HandBackTask>(takeover)?;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordChangesReading| command.task,
            |task: &mut Task, command: &RecordChangesReading, _| -> Result<(), TaskError> {
                task.changes_reading(command.job)
            },
        ))?;
        platform.command(EntityHandler::new(
            Arc::clone(store),
            Arc::clone(clock),
            |command: &RecordHandedBack| command.task,
            |task: &mut Task, command: &RecordHandedBack, _| -> Result<(), TaskError> {
                task.handed_back(command.prompt.clone())
            },
        ))?;
        Ok(())
    }
}
