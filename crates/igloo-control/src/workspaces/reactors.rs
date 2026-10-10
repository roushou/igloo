use async_trait::async_trait;
use igloo_core::sandbox::{Sandbox, SandboxEvent, SandboxPhase};
use igloo_core::seal::{Seal, SealEvent};
use igloo_core::{Actor, SystemComponent};

use super::commands::{
    RecordWorkspaceEnded, RecordWorkspaceRunning, RecordWorkspaceSeal, WorkspaceQueries,
};
use crate::app::{AppError, CommandBus, Reactor, RequestContext};
use crate::ports::EventEnvelope;

fn context(event: &EventEnvelope) -> RequestContext {
    RequestContext::caused_by(
        event,
        Actor::System {
            component: SystemComponent::Reactor,
        },
    )
}

/// Tells a workspace when its sandbox runs or ends.
pub(super) struct FollowSandboxes {
    pub(super) workspaces: WorkspaceQueries,
}

#[async_trait]
impl Reactor for FollowSandboxes {
    fn name(&self) -> &'static str {
        "workspace.follow_sandboxes"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let Some(sandbox) = event.subject_as::<Sandbox>() else {
            return Ok(());
        };
        let SandboxEvent::StatusRecorded { phase, .. } = event.decode::<SandboxEvent>()? else {
            return Ok(());
        };
        if phase != SandboxPhase::Running && !phase.is_terminal() {
            return Ok(());
        }
        let Some(workspace) = self.workspaces.with_sandbox(sandbox).await? else {
            return Ok(());
        };
        let workspace = igloo_core::Entity::id(&workspace);
        if phase == SandboxPhase::Running {
            let command = RecordWorkspaceRunning { workspace, sandbox };
            bus.dispatch(command, context(event)).await
        } else {
            let command = RecordWorkspaceEnded { workspace, sandbox };
            bus.dispatch(command, context(event)).await
        }
    }
}

/// Tells a workspace how its seal ended.
pub(super) struct FollowSeals {
    pub(super) workspaces: WorkspaceQueries,
}

#[async_trait]
impl Reactor for FollowSeals {
    fn name(&self) -> &'static str {
        "workspace.follow_seals"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let Some(seal) = event.subject_as::<Seal>() else {
            return Ok(());
        };
        let snapshot = match event.decode::<SealEvent>()? {
            SealEvent::Sealed { snapshot } => Ok(snapshot),
            SealEvent::Failed { .. } => Err(()),
            SealEvent::Requested { .. } => return Ok(()),
        };
        let Some(workspace) = self.workspaces.with_seal(seal).await? else {
            return Ok(());
        };
        let command = RecordWorkspaceSeal {
            workspace: igloo_core::Entity::id(&workspace),
            seal,
            snapshot,
        };
        bus.dispatch(command, context(event)).await
    }
}
