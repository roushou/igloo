use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use igloo_core::sandbox::{FailureReason, NetworkPolicy, ResourceLimits, SandboxId, SandboxPhase};
use igloo_core::{Digest, Generation};
use igloo_worker_protocol::v1;
use igloo_worker_protocol::v1::connect_request::Message as Uplink;
use tokio::task::JoinSet;

use crate::blobs::RemoteLayer;
use crate::layers::LayerCache;
use crate::rootfs::Rootfs;
use crate::runtime::{LocalSandbox, SandboxRuntime};
use crate::store::{LocalStore, SandboxRecord};

/// The sandboxes on this worker, converged to the latest assignment.
///
/// Starting a sandbox (downloading, unpacking and assembling its layers) runs in the
/// background; [`Sandboxes::next_started`] yields the reports once it is done. Every sandbox,
/// starting or present, pins its layers in the cache.
pub(crate) struct Sandboxes {
    local: HashMap<SandboxId, Present>,
    adopted: HashSet<SandboxId>,
    starting: HashMap<SandboxId, Generation>,
    in_flight: HashSet<SandboxId>,
    tasks: JoinSet<Started>,
    setup: Setup,
}

/// A sandbox ready for jobs.
struct Present {
    sandbox: LocalSandbox,
    layers: Vec<Digest>,
}

/// What starting a sandbox needs; cloned into each start.
#[derive(Clone)]
struct Setup {
    runtime: Arc<dyn SandboxRuntime>,
    layers: LayerCache,
    rootfs: Rootfs,
    store: LocalStore,
}

/// The end of one start.
pub(crate) struct Started {
    id: SandboxId,
    layers: Vec<Digest>,
    outcome: Result<LocalSandbox, FailureReason>,
}

impl Sandboxes {
    /// Sandboxes recorded before a restart are adopted: kept as they are and reported running
    /// with the next assignment that still lists them.
    pub(crate) async fn adopt(
        runtime: Arc<dyn SandboxRuntime>,
        layers: LayerCache,
        rootfs: Rootfs,
        store: LocalStore,
    ) -> std::io::Result<Self> {
        let mut local = HashMap::new();
        for record in store.records().await? {
            layers.pin(&record.layers);
            let sandbox = LocalSandbox {
                id: record.id,
                generation: record.generation,
                root: store.rootfs(record.id),
                env: record.env,
                limits: record.limits,
                network: record.network,
            };
            local.insert(
                record.id,
                Present {
                    sandbox,
                    layers: record.layers,
                },
            );
        }
        Ok(Self {
            adopted: local.keys().copied().collect(),
            local,
            starting: HashMap::new(),
            in_flight: HashSet::new(),
            tasks: JoinSet::new(),
            setup: Setup {
                runtime,
                layers,
                rootfs,
                store,
            },
        })
    }

    /// The sandbox `id`, if it is ready.
    pub(crate) fn get(&self, id: SandboxId) -> Option<&LocalSandbox> {
        self.local.get(&id).map(|present| &present.sandbox)
    }

    /// Whether sandbox `id` is still starting.
    pub(crate) fn is_starting(&self, id: SandboxId) -> bool {
        self.starting.contains_key(&id)
    }

    /// Whether any start is in progress, including ones whose sandbox was removed meanwhile.
    pub(crate) fn has_tasks(&self) -> bool {
        !self.tasks.is_empty()
    }

    /// Converges to `assignment`, returning the status reports to send now. Starts begun here
    /// report through [`Sandboxes::next_started`].
    pub(crate) async fn apply(&mut self, assignment: &v1::Assignment) -> Vec<Uplink> {
        let mut reports = Vec::new();
        let mut assigned = HashSet::new();
        for sandbox in &assignment.sandboxes {
            let (Ok(id), Ok(generation)) = (
                sandbox.sandbox_id.parse::<SandboxId>(),
                Generation::try_from(sandbox.generation),
            ) else {
                tracing::warn!(
                    sandbox = sandbox.sandbox_id,
                    "ignoring malformed assignment entry"
                );
                continue;
            };
            assigned.insert(id);
            match sandbox.desired() {
                v1::DesiredState::Stopped => self.stop(id, generation, &mut reports).await,
                _ => self.run(sandbox, id, generation, &mut reports),
            }
        }
        let gone: Vec<SandboxId> = self
            .local
            .keys()
            .chain(self.starting.keys())
            .filter(|id| !assigned.contains(id))
            .copied()
            .collect();
        for id in gone {
            self.starting.remove(&id);
            self.remove(id).await;
        }
        reports
    }

    /// Waits for the next start to end; `None` once none is left or when one did not finish.
    /// Cancel-safe: dropping the wait loses nothing.
    pub(crate) async fn next_started(&mut self) -> Option<Started> {
        match self.tasks.join_next().await? {
            Ok(started) => Some(started),
            Err(error) => {
                tracing::warn!(%error, "a sandbox start did not finish");
                None
            }
        }
    }

    /// Records an ended start and returns its reports: running, failed, or none when the
    /// sandbox was removed while it started, in which case it is cleaned up.
    pub(crate) async fn record_started(&mut self, started: Started) -> Vec<Uplink> {
        let Started {
            id,
            layers,
            outcome,
        } = started;
        self.in_flight.remove(&id);
        let Some(generation) = self.starting.remove(&id) else {
            if let Ok(sandbox) = &outcome
                && let Err(error) = self.setup.runtime.stop(sandbox).await
            {
                tracing::warn!(%id, %error, "stopping the sandbox failed");
            }
            self.setup.clean_up(id, &layers).await;
            return Vec::new();
        };
        match outcome {
            Ok(mut sandbox) => {
                sandbox.generation = generation;
                self.local.insert(id, Present { sandbox, layers });
                if let Err(error) = self.setup.layers.evict().await {
                    tracing::warn!(%error, "evicting cached layers failed");
                }
                vec![Self::report(id, SandboxPhase::Running, generation)]
            }
            Err(reason) => {
                self.setup.clean_up(id, &layers).await;
                vec![Self::report(
                    id,
                    SandboxPhase::Failed { reason },
                    generation,
                )]
            }
        }
    }

    fn run(
        &mut self,
        assigned: &v1::AssignedSandbox,
        id: SandboxId,
        generation: Generation,
        reports: &mut Vec<Uplink>,
    ) {
        if let Some(present) = self.local.get_mut(&id) {
            if self.adopted.remove(&id) || present.sandbox.generation != generation {
                present.sandbox.generation = generation;
                reports.push(Self::report(id, SandboxPhase::Running, generation));
            }
            return;
        }
        if self.in_flight.contains(&id) {
            // Still starting, or removed and assigned again before its start ended: the start
            // in flight serves the latest generation.
            self.starting.insert(id, generation);
            return;
        }
        reports.push(Self::report(id, SandboxPhase::Starting, generation));
        let Ok(remote) = assigned
            .layers
            .iter()
            .map(RemoteLayer::try_from)
            .collect::<Result<Vec<_>, _>>()
        else {
            let reason = FailureReason::SnapshotUnavailable;
            reports.push(Self::report(
                id,
                SandboxPhase::Failed { reason },
                generation,
            ));
            return;
        };
        let layers: Vec<Digest> = remote.iter().map(|layer| layer.layer().digest()).collect();
        self.setup.layers.pin(&layers);
        self.starting.insert(id, generation);
        self.in_flight.insert(id);
        let sandbox = LocalSandbox {
            id,
            generation,
            root: self.setup.store.rootfs(id),
            env: assigned.env.clone().into_iter().collect(),
            // Values the server would not send fall back to the tightest settings.
            limits: ResourceLimits::new(assigned.millicpus, assigned.memory_mib)
                .unwrap_or_default(),
            network: NetworkPolicy::try_from(assigned.network()).unwrap_or_default(),
        };
        let setup = self.setup.clone();
        self.tasks.spawn(async move {
            let outcome = setup.start(sandbox, &remote).await;
            Started {
                id,
                layers,
                outcome,
            }
        });
    }

    async fn stop(&mut self, id: SandboxId, generation: Generation, reports: &mut Vec<Uplink>) {
        if self.local.contains_key(&id) || self.starting.remove(&id).is_some() {
            reports.push(Self::report(id, SandboxPhase::Stopping, generation));
            self.remove(id).await;
        }
        reports.push(Self::report(id, SandboxPhase::Stopped, generation));
    }

    /// Removes a present sandbox. One still starting is cleaned up when its start ends.
    async fn remove(&mut self, id: SandboxId) {
        self.adopted.remove(&id);
        let Some(present) = self.local.remove(&id) else {
            return;
        };
        if let Err(error) = self.setup.runtime.stop(&present.sandbox).await {
            tracing::warn!(%id, %error, "stopping the sandbox failed");
        }
        self.setup.clean_up(id, &present.layers).await;
    }

    fn report(id: SandboxId, phase: SandboxPhase, generation: Generation) -> Uplink {
        Uplink::SandboxStatus(v1::SandboxStatus::new(&id.to_string(), phase, generation))
    }
}

impl Setup {
    /// Caches the layers, assembles the file system, starts the runtime and records the
    /// sandbox.
    async fn start(
        &self,
        sandbox: LocalSandbox,
        remote: &[RemoteLayer],
    ) -> Result<LocalSandbox, FailureReason> {
        let id = sandbox.id;
        if remote.is_empty() {
            return Err(FailureReason::SnapshotUnavailable);
        }
        let mut paths = Vec::with_capacity(remote.len());
        for layer in remote {
            match self.layers.ensure(layer).await {
                Ok(path) => paths.push(path),
                Err(error) => {
                    tracing::warn!(%id, %error, "caching a layer failed");
                    return Err(FailureReason::SnapshotUnavailable);
                }
            }
        }
        let rootfs = self.rootfs;
        let sandbox_dir = self.store.sandbox_dir(id);
        let assembled =
            tokio::task::spawn_blocking(move || rootfs.assemble(&paths, &sandbox_dir)).await;
        if let Err(error) = assembled
            .map_err(std::io::Error::other)
            .and_then(|result| result)
        {
            tracing::warn!(%id, %error, "assembling the file system failed");
            return Err(FailureReason::SnapshotUnavailable);
        }
        if let Err(error) = self.runtime.start(&sandbox).await {
            tracing::warn!(%id, %error, "starting the sandbox failed");
            return Err(FailureReason::RuntimeError);
        }
        let record = SandboxRecord {
            id,
            generation: sandbox.generation,
            env: sandbox.env.clone(),
            layers: remote.iter().map(|layer| layer.layer().digest()).collect(),
            limits: sandbox.limits,
            network: sandbox.network,
        };
        if let Err(error) = self.store.save_record(&record).await {
            tracing::warn!(%id, %error, "recording the sandbox failed");
            return Err(FailureReason::RuntimeError);
        }
        Ok(sandbox)
    }

    /// Releases the file system, deletes the sandbox's files and unpins its layers.
    async fn clean_up(&self, id: SandboxId, layers: &[Digest]) {
        if let Err(error) = self.rootfs.release(&self.store.sandbox_dir(id)) {
            tracing::warn!(%id, %error, "releasing the file system failed");
        }
        if let Err(error) = self.store.remove_sandbox(id).await {
            tracing::warn!(%id, %error, "removing the sandbox failed");
        }
        self.layers.unpin(layers);
    }
}
