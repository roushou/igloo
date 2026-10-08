use std::sync::Arc;

use async_trait::async_trait;
use igloo_core::process::EnvVars;
use igloo_core::sandbox::{Sandbox, SandboxEvent, SandboxId, SandboxPhase};
use igloo_core::seal::{Seal, SealFailure, SealId, SealPhase};
use igloo_core::snapshot::{SnapshotId, SnapshotLayer};
use igloo_core::{Actor, Entity, ErrorCode, Resource, SystemComponent};

use super::Snapshots;
use crate::app::{
    AppError, Command, CommandBus, CommandHandler, EntityHandler, Extension, InstallError,
    PlatformBuilder, Reactor, RequestContext,
};
use crate::ports::{
    Clock, EntityStore, EventEnvelope, IdGenerator, IdGeneratorExt, StorageError, Versioned,
};

/// Requests a seal of a running sandbox.
pub struct CreateSeal {
    /// The sandbox.
    pub sandbox: SandboxId,
}

/// Completes a seal with its uploaded layer: the snapshot is the base snapshot's layers plus
/// the layer, or the layer alone when it holds the whole root file system.
pub struct CompleteSeal {
    /// The seal.
    pub seal: SealId,
    /// The stored layer.
    pub layer: SnapshotLayer,
    /// Whether the layer holds the whole root file system.
    pub full: bool,
}

/// Fails a pending seal.
pub struct FailSeal {
    /// The seal.
    pub seal: SealId,
    /// Why.
    pub reason: SealFailure,
}

impl Command for CreateSeal {
    type Output = SealId;
    const NAME: &'static str = "seal.create";
}

impl Command for CompleteSeal {
    type Output = SnapshotId;
    const NAME: &'static str = "seal.complete";
}

impl Command for FailSeal {
    type Output = ();
    const NAME: &'static str = "seal.fail";
}

/// Why a seal cannot be requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CreateSealError {
    /// Only a running sandbox can be sealed.
    #[error("the sandbox is not running")]
    SandboxNotRunning,
}

impl ErrorCode for CreateSealError {
    fn code(&self) -> &'static str {
        match self {
            Self::SandboxNotRunning => "seal.sandbox_not_running",
        }
    }
}

/// Read access to seals beyond loading one by id.
#[derive(Clone)]
pub struct SealQueries {
    store: Arc<dyn EntityStore<Seal>>,
}

impl SealQueries {
    /// Queries over `store`.
    #[must_use]
    pub fn new(store: Arc<dyn EntityStore<Seal>>) -> Self {
        Self { store }
    }

    /// Pending seals of any of `sandboxes`, oldest first.
    pub async fn pending_in(&self, sandboxes: &[SandboxId]) -> Result<Vec<Seal>, StorageError> {
        let mut pending: Vec<Seal> = self
            .store
            .all()
            .await?
            .into_iter()
            .filter(|seal| {
                seal.phase() == SealPhase::Pending && sandboxes.contains(&seal.sandbox())
            })
            .collect();
        pending.sort_by_key(Entity::id);
        Ok(pending)
    }
}

/// Creates a seal after checking its sandbox runs.
struct CreateSealHandler {
    seals: Arc<dyn EntityStore<Seal>>,
    sandboxes: Arc<dyn EntityStore<Sandbox>>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
}

#[async_trait]
impl CommandHandler<CreateSeal> for CreateSealHandler {
    async fn handle(
        &self,
        command: CreateSeal,
        context: &RequestContext,
    ) -> Result<SealId, AppError> {
        let sandbox = self
            .sandboxes
            .load(command.sandbox)
            .await?
            .ok_or_else(|| AppError::not_found(Sandbox::NAME, &command.sandbox))?;
        let sandbox = sandbox.entity();
        if sandbox.status().phase() != SandboxPhase::Running {
            return Err(AppError::domain(&CreateSealError::SandboxNotRunning));
        }
        let id = self.ids.next::<Seal>();
        let now = self.clock.now();
        let mut seal = Versioned::new(Seal::new(id, sandbox.id(), sandbox.spec().snapshot(), now));
        self.seals
            .commit(&mut seal, &context.commit_meta(now))
            .await?;
        Ok(id)
    }
}

/// Registers the sealed snapshot, then records it on the seal.
struct CompleteSealHandler {
    seals: Arc<dyn EntityStore<Seal>>,
    snapshots: Snapshots,
    clock: Arc<dyn Clock>,
}

#[async_trait]
impl CommandHandler<CompleteSeal> for CompleteSealHandler {
    async fn handle(
        &self,
        command: CompleteSeal,
        context: &RequestContext,
    ) -> Result<SnapshotId, AppError> {
        let mut seal = self
            .seals
            .load(command.seal)
            .await?
            .ok_or_else(|| AppError::not_found(Seal::NAME, &command.seal))?;
        let base = seal.entity().base();
        let snapshot = if command.full {
            self.snapshots.replace(base, vec![command.layer]).await?
        } else {
            self.snapshots
                .extend(Some(base), vec![command.layer], &EnvVars::default())
                .await?
        };
        seal.entity_mut()
            .complete(snapshot)
            .map_err(|error| AppError::domain(&error))?;
        self.seals
            .commit(&mut seal, &context.commit_meta(self.clock.now()))
            .await?;
        Ok(snapshot)
    }
}

/// Fails the pending seals of a sandbox once it stops or fails.
struct FailSealsOfEndedSandboxes {
    queries: SealQueries,
}

#[async_trait]
impl Reactor for FailSealsOfEndedSandboxes {
    fn name(&self) -> &'static str {
        "seal.fail_on_sandbox_end"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let Some(sandbox) = event.subject_as::<Sandbox>() else {
            return Ok(());
        };
        let ended = matches!(
            event.decode::<SandboxEvent>()?,
            SandboxEvent::StatusRecorded { phase, .. } if phase.is_terminal()
        );
        if !ended {
            return Ok(());
        }
        let actor = Actor::System {
            component: SystemComponent::Reactor,
        };
        for seal in self.queries.pending_in(&[sandbox]).await? {
            let command = FailSeal {
                seal: seal.id(),
                reason: SealFailure::SandboxEnded,
            };
            bus.dispatch(command, RequestContext::caused_by(event, actor))
                .await?;
        }
        Ok(())
    }
}

/// Seals: requests, completion by upload, and failure when their sandbox ends.
pub struct SealModule;

impl Extension for SealModule {
    fn name(&self) -> &'static str {
        "seal"
    }

    fn install(self: Box<Self>, platform: &mut PlatformBuilder) -> Result<(), InstallError> {
        let seals = platform.store::<Seal>()?;
        let ports = platform.ports().clone();
        platform.command(CreateSealHandler {
            seals: Arc::clone(&seals),
            sandboxes: platform.store::<Sandbox>()?,
            clock: Arc::clone(&ports.clock),
            ids: Arc::clone(&ports.ids),
        })?;
        platform.command(CompleteSealHandler {
            seals: Arc::clone(&seals),
            snapshots: Snapshots::new(Arc::clone(&ports.blobs)),
            clock: Arc::clone(&ports.clock),
        })?;
        platform.command(EntityHandler::infallible(
            Arc::clone(&seals),
            Arc::clone(&ports.clock),
            |command: &FailSeal| command.seal,
            |seal: &mut Seal, command: &FailSeal, _| seal.fail(command.reason),
        ))?;
        platform.reactor(FailSealsOfEndedSandboxes {
            queries: SealQueries::new(seals),
        });
        Ok(())
    }
}
