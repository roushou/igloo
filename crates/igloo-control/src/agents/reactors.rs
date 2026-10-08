use async_trait::async_trait;
use igloo_core::build::{Build, BuildEvent, BuildOutcome};
use igloo_core::change::{Change, ChangeEvent};
use igloo_core::job::{Job, JobEvent};
use igloo_core::{Actor, Entity, SystemComponent};

use super::commands::{
    CancelTask, FinishTask, RecordTaskBuilt, RecordTurnEnded, RequestTurn, TaskQueries,
};
use super::task::{Task, TaskEvent};
use crate::app::{AppError, CommandBus, Reactor, RequestContext};
use crate::ci::{AgentWork, AttributeOutcome, Outcome, OutcomeEvent};
use crate::platform::BuildQueries;
use crate::ports::EventEnvelope;

/// Records how the turns of tasks ended.
pub(super) struct RecordTurns {
    pub(super) tasks: TaskQueries,
}

/// Records how the builds tasks wait on ended, whichever of the build's end and the task's
/// wait is recorded first.
pub(super) struct RecordTaskBuilds {
    pub(super) tasks: TaskQueries,
    pub(super) builds: BuildQueries,
}

/// Follows the reviews of tasks' changes: a request for changes starts the task's next turn, a
/// merge finishes the task and a close cancels it.
pub(super) struct FollowReviews {
    pub(super) tasks: TaskQueries,
}

#[async_trait]
impl Reactor for FollowReviews {
    fn name(&self) -> &'static str {
        "agents.follow_reviews"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let Some(change) = event.subject_as::<Change>() else {
            return Ok(());
        };
        let decoded = event.decode::<ChangeEvent>()?;
        if !matches!(
            decoded,
            ChangeEvent::ChangesRequested { .. } | ChangeEvent::Merged { .. } | ChangeEvent::Closed
        ) {
            return Ok(());
        }
        let Some(task) = self.tasks.of_change(change).await? else {
            return Ok(());
        };
        let task = task.id();
        match decoded {
            ChangeEvent::ChangesRequested { request } => {
                let prompt = Task::review_prompt(&request);
                bus.dispatch(RequestTurn { task, prompt }, context(event))
                    .await
            }
            ChangeEvent::Merged { .. } => bus.dispatch(FinishTask { task }, context(event)).await,
            _ => bus.dispatch(CancelTask { task }, context(event)).await,
        }
    }
}

/// Names the task, tool and turns behind the outcome of every change a task made.
pub(super) struct AttributeOutcomes {
    pub(super) tasks: TaskQueries,
}

#[async_trait]
impl Reactor for AttributeOutcomes {
    fn name(&self) -> &'static str {
        "agents.attribute_outcomes"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let Some(outcome) = event.subject_as::<Outcome>() else {
            return Ok(());
        };
        let OutcomeEvent::Recorded { record, .. } = event.decode::<OutcomeEvent>()? else {
            return Ok(());
        };
        let Some(task) = self.tasks.of_change(record.change).await? else {
            return Ok(());
        };
        let Some(prepared) = task.settings() else {
            return Ok(());
        };
        let work = AgentWork {
            task: task.id().to_string(),
            tool: prepared.tool.to_string(),
            turns: u32::try_from(task.turns().len()).unwrap_or(u32::MAX),
        };
        bus.dispatch(AttributeOutcome { outcome, work }, context(event))
            .await
    }
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
impl Reactor for RecordTurns {
    fn name(&self) -> &'static str {
        "agents.record_turns"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let Some(job) = event.subject_as::<Job>() else {
            return Ok(());
        };
        let Some(ending) = event.decode::<JobEvent>()?.ending() else {
            return Ok(());
        };
        let Some(task) = self.tasks.with_job(job).await? else {
            return Ok(());
        };
        let command = RecordTurnEnded {
            task: task.id(),
            job,
            ending,
        };
        bus.dispatch(command, context(event)).await
    }
}

#[async_trait]
impl Reactor for RecordTaskBuilds {
    fn name(&self) -> &'static str {
        "agents.record_task_builds"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let (build, tasks) = if let Some(build) = event.subject_as::<Build>() {
            if !matches!(
                event.decode::<BuildEvent>()?,
                BuildEvent::Recorded | BuildEvent::Failed { .. }
            ) {
                return Ok(());
            }
            (build, self.tasks.of_build(build).await?)
        } else if let Some(task) = event.subject_as::<Task>() {
            let TaskEvent::Building { build } = event.decode::<TaskEvent>()? else {
                return Ok(());
            };
            (build, self.tasks.get(task).await?.into_iter().collect())
        } else {
            return Ok(());
        };
        let Some(found) = self.builds.get(build).await? else {
            return Ok(());
        };
        let result = match found.outcome() {
            Some(BuildOutcome::Built { .. }) => Ok(()),
            Some(BuildOutcome::Failed { reason }) => Err(reason.clone()),
            None => return Ok(()),
        };
        for task in tasks {
            let command = RecordTaskBuilt {
                task: task.id(),
                build,
                result: result.clone(),
            };
            bus.dispatch(command, context(event)).await?;
        }
        Ok(())
    }
}
