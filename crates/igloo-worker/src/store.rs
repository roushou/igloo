use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use igloo_core::sandbox::{NetworkPolicy, ResourceLimits, SandboxId};
use igloo_core::{Digest, Generation};
use igloo_worker_protocol::v1;
use prost::Message;
use serde::{Deserialize, Serialize};

/// The worker's data directory:
///
/// ```text
/// worker-id                     the id from the last Welcome
/// sandboxes/<id>/record.json    what the worker knows about each sandbox
/// sandboxes/<id>/rootfs/        the sandbox's file system (an overlay mount point on Linux)
/// sandboxes/<id>/upper/, work/  the overlay's private directories
/// layers/, downloads/           the layer cache (see `LayerCache`)
/// outbox/<job>.pb               results not acknowledged yet
/// ```
#[derive(Clone, Debug)]
pub(crate) struct LocalStore {
    root: PathBuf,
}

/// What survives a worker restart about one sandbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SandboxRecord {
    pub(crate) id: SandboxId,
    pub(crate) generation: Generation,
    pub(crate) env: BTreeMap<String, String>,
    /// The snapshot layers the sandbox was built from, lowest first.
    #[serde(default)]
    pub(crate) layers: Vec<Digest>,
    #[serde(default)]
    pub(crate) limits: ResourceLimits,
    #[serde(default)]
    pub(crate) network: NetworkPolicy,
}

impl LocalStore {
    pub(crate) async fn open(root: PathBuf) -> io::Result<Self> {
        for dir in ["sandboxes", "outbox"] {
            tokio::fs::create_dir_all(root.join(dir)).await?;
        }
        Ok(Self { root })
    }

    pub(crate) async fn worker_id(&self) -> Option<String> {
        let id = tokio::fs::read_to_string(self.root.join("worker-id"))
            .await
            .ok()?;
        Some(id.trim().to_owned()).filter(|id| !id.is_empty())
    }

    pub(crate) async fn set_worker_id(&self, id: &str) -> io::Result<()> {
        tokio::fs::write(self.root.join("worker-id"), id).await
    }

    pub(crate) async fn clear_worker_id(&self) -> io::Result<()> {
        match tokio::fs::remove_file(self.root.join("worker-id")).await {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    pub(crate) fn sandbox_dir(&self, id: SandboxId) -> PathBuf {
        self.root.join("sandboxes").join(id.to_string())
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn rootfs(&self, id: SandboxId) -> PathBuf {
        self.sandbox_dir(id).join("rootfs")
    }

    pub(crate) async fn save_record(&self, record: &SandboxRecord) -> io::Result<()> {
        let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
        Self::write_atomically(&self.sandbox_dir(record.id).join("record.json"), &bytes).await
    }

    /// Every sandbox recorded before a restart.
    pub(crate) async fn records(&self) -> io::Result<Vec<SandboxRecord>> {
        let mut records = Vec::new();
        let mut entries = tokio::fs::read_dir(self.root.join("sandboxes")).await?;
        while let Some(entry) = entries.next_entry().await? {
            let Ok(bytes) = tokio::fs::read(entry.path().join("record.json")).await else {
                continue;
            };
            if let Ok(record) = serde_json::from_slice(&bytes) {
                records.push(record);
            }
        }
        Ok(records)
    }

    pub(crate) async fn remove_sandbox(&self, id: SandboxId) -> io::Result<()> {
        match tokio::fs::remove_dir_all(self.sandbox_dir(id)).await {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    pub(crate) async fn save_result(&self, result: &v1::JobResult) -> io::Result<()> {
        let path = self.outbox_path(&result.job_id);
        Self::write_atomically(&path, &result.encode_to_vec()).await
    }

    pub(crate) async fn remove_result(&self, job_id: &str) -> io::Result<()> {
        match tokio::fs::remove_file(self.outbox_path(job_id)).await {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// Every result still waiting for an acknowledgement.
    pub(crate) async fn pending_results(&self) -> io::Result<Vec<v1::JobResult>> {
        let mut results = Vec::new();
        let mut entries = tokio::fs::read_dir(self.root.join("outbox")).await?;
        while let Some(entry) = entries.next_entry().await? {
            let bytes = tokio::fs::read(entry.path()).await?;
            if let Ok(result) = v1::JobResult::decode(bytes.as_slice()) {
                results.push(result);
            }
        }
        Ok(results)
    }

    fn outbox_path(&self, job_id: &str) -> PathBuf {
        self.root.join("outbox").join(format!("{job_id}.pb"))
    }

    /// Writes through a temporary file, so a crash never leaves a partial file.
    pub(crate) async fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let temporary = path.with_extension("tmp");
        tokio::fs::write(&temporary, bytes).await?;
        tokio::fs::rename(&temporary, path).await
    }
}
