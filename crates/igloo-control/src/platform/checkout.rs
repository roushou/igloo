use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use igloo_core::Digest;
use igloo_core::process::EnvVars;
use igloo_core::repo::{CommitId, RepoId, WarmSnapshot};
use igloo_core::snapshot::{MediaType, SnapshotId, SnapshotLayer};

use super::Snapshots;
use crate::app::AppError;
use crate::ports::{BlobStore, Forge, ForgeError, StorageError};

/// Snapshots of repositories at commits: a base snapshot (such as an imported image) and a
/// layer holding the checkout under `workspace/`, with a shallow `.git` at the commit. Over a
/// warm snapshot, the layer also deletes the files removed since the warm snapshot's commit and
/// replaces its `.git`. Layer entries keep their files' modes and modification times, which
/// build tools compare against warm build output, and are owned by root whoever the server runs
/// as.
#[derive(Clone)]
pub struct RepoSnapshots {
    forge: Arc<dyn Forge>,
    blobs: Arc<dyn BlobStore>,
    snapshots: Snapshots,
}

/// How a repository's warm snapshots are built and keyed: a command run over a checkout, whose
/// result depends only on the base snapshot, the command and the lockfiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarmRecipe {
    /// The build command, such as `cargo build --tests`.
    pub command: String,
    /// Paths of the files the build's dependencies are pinned in, such as `Cargo.lock`.
    pub lockfiles: Vec<String>,
}

impl RepoSnapshots {
    const ROOT: &'static str = "workspace";
    const WHITEOUT: &'static str = ".wh.";
    const OPAQUE: &'static str = ".wh..wh..opq";

    /// Snapshots from `forge`'s mirrors, stored in `blobs`.
    #[must_use]
    pub fn new(forge: Arc<dyn Forge>, blobs: Arc<dyn BlobStore>) -> Self {
        Self {
            snapshots: Snapshots::new(Arc::clone(&blobs)),
            forge,
            blobs,
        }
    }

    /// The key of the warm snapshot `recipe` builds for `repo` at `commit` over `base`.
    pub async fn warm_key(
        &self,
        repo: RepoId,
        commit: &CommitId,
        base: Option<SnapshotId>,
        recipe: &WarmRecipe,
    ) -> Result<Digest, AppError> {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"igloo.warm.v1\n");
        let base = base.map_or_else(|| "-".to_owned(), |base| base.to_string());
        hasher.update(format!("base {base}\ncommand {}\n", recipe.command).as_bytes());
        for path in &recipe.lockfiles {
            let content = self.forge.read_file(repo, commit, path).await?;
            let digest = content.map_or_else(
                || "absent".to_owned(),
                |bytes| blake3::hash(&bytes).to_hex().to_string(),
            );
            hasher.update(format!("file {path} {digest}\n").as_bytes());
        }
        Ok(Digest::from_blake3(*hasher.finalize().as_bytes()))
    }

    /// Registers the snapshot of `repo` at `commit`: over `warm` when given, else over `base`.
    pub async fn checkout(
        &self,
        repo: RepoId,
        commit: &CommitId,
        base: Option<SnapshotId>,
        warm: Option<&WarmSnapshot>,
    ) -> Result<SnapshotId, AppError> {
        let deleted = match warm {
            Some(warm) => self.forge.deleted_paths(repo, &warm.commit, commit).await?,
            None => Vec::new(),
        };
        let scratch = tempfile::tempdir().map_err(storage)?;
        let tree = scratch.path().join("tree");
        self.forge.checkout(repo, commit, &tree).await?;
        let archive = scratch.path().join("layer.tar");
        let replaces_git = warm.is_some();
        let (target, source) = (archive.clone(), tree.clone());
        let digest = tokio::task::spawn_blocking(move || {
            Self::pack(&source, &deleted, replaces_git, &target)
        })
        .await
        .map_err(storage)?
        .map_err(storage)?;
        if !self.blobs.contains(digest).await? {
            let file = tokio::fs::File::open(&archive).await.map_err(storage)?;
            self.blobs.put(digest, Box::pin(file)).await?;
        }
        let layer = SnapshotLayer::new(digest, MediaType::Tar);
        let under = warm.map(|warm| warm.snapshot).or(base);
        self.snapshots
            .extend(under, vec![layer], &EnvVars::default())
            .await
    }

    /// Writes `tree` under `workspace/` to `archive`, adding whiteouts for `deleted` paths and,
    /// when `replaces_git`, an opaque marker hiding a lower `.git`; returns the archive's digest.
    fn pack(
        tree: &Path,
        deleted: &[String],
        replaces_git: bool,
        archive: &Path,
    ) -> io::Result<Digest> {
        let file = std::fs::File::create(archive)?;
        let mut builder = tar::Builder::new(io::BufWriter::new(file));
        Self::append(&mut builder, Path::new(Self::ROOT), tree)?;
        for path in deleted {
            let path = Path::new(path);
            let Some(name) = path.file_name() else {
                continue;
            };
            let mut marker = std::ffi::OsString::from(Self::WHITEOUT);
            marker.push(name);
            let parent = path.parent().unwrap_or_else(|| Path::new(""));
            Self::marker(
                &mut builder,
                &Path::new(Self::ROOT).join(parent).join(marker),
            )?;
        }
        if replaces_git {
            let opaque = Path::new(Self::ROOT).join(".git").join(Self::OPAQUE);
            Self::marker(&mut builder, &opaque)?;
        }
        builder
            .into_inner()?
            .into_inner()
            .map_err(io::Error::other)?
            .sync_all()?;
        let mut hasher = blake3::Hasher::new();
        hasher.update_reader(std::fs::File::open(archive)?)?;
        Ok(Digest::from_blake3(*hasher.finalize().as_bytes()))
    }

    /// Appends `path` as `name`, and a directory's entries beneath it in name order, owned by
    /// root. Symbolic links are stored as links.
    fn append(
        builder: &mut tar::Builder<impl io::Write>,
        name: &Path,
        path: &Path,
    ) -> io::Result<()> {
        let metadata = std::fs::symlink_metadata(path)?;
        let mut header = tar::Header::new_gnu();
        header.set_metadata_in_mode(&metadata, tar::HeaderMode::Complete);
        header.set_uid(0);
        header.set_gid(0);
        header.set_username("root")?;
        header.set_groupname("root")?;
        if metadata.is_dir() {
            header.set_size(0);
            builder.append_data(&mut header, name, io::empty())?;
            let mut entries = std::fs::read_dir(path)?.collect::<io::Result<Vec<_>>>()?;
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                Self::append(builder, &name.join(entry.file_name()), &entry.path())?;
            }
            Ok(())
        } else if metadata.file_type().is_symlink() {
            header.set_size(0);
            builder.append_link(&mut header, name, std::fs::read_link(path)?)
        } else {
            builder.append_data(&mut header, name, std::fs::File::open(path)?)
        }
    }

    fn marker(builder: &mut tar::Builder<impl io::Write>, path: &PathBuf) -> io::Result<()> {
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o600);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_entry_type(tar::EntryType::Regular);
        builder.append_data(&mut header, path, io::empty())
    }
}

impl From<ForgeError> for AppError {
    fn from(error: ForgeError) -> Self {
        match error {
            ForgeError::BranchNotFound(branch) => Self::not_found("branch", &branch),
            ForgeError::CommitNotFound(commit) => Self::not_found("commit", &commit),
            ForgeError::Moved(_) => Self::domain(&error),
            ForgeError::Git(_) => Self::infrastructure(error),
        }
    }
}

impl igloo_core::ErrorCode for ForgeError {
    fn code(&self) -> &'static str {
        match self {
            Self::BranchNotFound(_) => "branch.not_found",
            Self::CommitNotFound(_) => "commit.not_found",
            Self::Moved(_) => "branch.moved",
            Self::Git(_) => "forge.failed",
        }
    }
}

fn storage(error: impl std::error::Error + Send + Sync + 'static) -> AppError {
    AppError::from(StorageError::backend(error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn layers_are_owned_by_root_and_keep_modes_times_and_links() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("dir");
        let tree = dir.path().join("tree");
        std::fs::create_dir_all(tree.join("bin")).expect("tree");
        std::fs::write(tree.join("README"), "read me").expect("file");
        std::fs::write(tree.join("bin/run"), "#!/bin/sh\n").expect("script");
        std::fs::set_permissions(tree.join("bin/run"), std::fs::Permissions::from_mode(0o700))
            .expect("chmod");
        std::os::unix::fs::symlink("run", tree.join("bin/alias")).expect("symlink");
        let written = std::fs::metadata(tree.join("README"))
            .expect("metadata")
            .modified()
            .expect("mtime")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after the epoch")
            .as_secs();
        let archive = dir.path().join("layer.tar");
        RepoSnapshots::pack(&tree, &[], false, &archive).expect("pack");

        let mut entries = tar::Archive::new(std::fs::File::open(&archive).expect("open"));
        let mut modes = std::collections::BTreeMap::new();
        let mut times = std::collections::BTreeMap::new();
        let mut links = std::collections::BTreeMap::new();
        for entry in entries.entries().expect("entries") {
            let entry = entry.expect("entry");
            let header = entry.header();
            assert_eq!(
                (header.uid().expect("uid"), header.gid().expect("gid")),
                (0, 0)
            );
            let path = entry.path().expect("path").display().to_string();
            modes.insert(path.clone(), header.mode().expect("mode") & 0o777);
            times.insert(path.clone(), header.mtime().expect("mtime"));
            if let Some(target) = entry.link_name().expect("link") {
                links.insert(path, target.display().to_string());
            }
        }
        assert_eq!(modes.get("workspace/bin/run"), Some(&0o700));
        assert_eq!(times.get("workspace/README"), Some(&written), "real times");
        assert_eq!(
            links.get("workspace/bin/alias").map(String::as_str),
            Some("run")
        );
        assert!(modes.contains_key("workspace"), "the root directory");
    }
}
