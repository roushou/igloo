use std::io;
use std::path::{Path, PathBuf};

use igloo_core::sandbox::SandboxId;

/// The network namespaces of sandboxes, one per sandbox, each kept alive by a bind mount at
/// `<dir>/<sandbox id>` so every container of the sandbox joins the same one.
///
/// Invariant: a namespace this type created holds only the loopback interface, and it is up.
pub(super) struct NetworkNamespaces {
    dir: PathBuf,
}

impl NetworkNamespaces {
    /// Namespaces mounted under `dir`.
    pub(super) const fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Where `sandbox`'s namespace is mounted; containers join it by this path.
    pub(super) fn path(&self, sandbox: SandboxId) -> PathBuf {
        self.dir.join(sandbox.to_string())
    }

    /// Creates `sandbox`'s namespace with loopback up, replacing one left by an earlier run.
    pub(super) async fn create(&self, sandbox: SandboxId) -> io::Result<()> {
        tokio::fs::create_dir_all(&self.dir).await?;
        let path = self.path(sandbox);
        Self::release(&path)?;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        // A dedicated thread: entering a namespace changes the calling thread for good, so no
        // runtime thread may do it.
        std::thread::Builder::new()
            .name("igloo-netns".to_owned())
            .spawn(move || {
                let created = sys::create(&path);
                if created.is_err() {
                    let _ = Self::release(&path);
                }
                let _ = sender.send(created);
            })?;
        receiver
            .await
            .map_err(|_| io::Error::other("creating the network namespace stopped"))?
    }

    /// Removes `sandbox`'s namespace, if it has one.
    pub(super) fn remove(&self, sandbox: SandboxId) -> io::Result<()> {
        Self::release(&self.path(sandbox))
    }

    /// Unmounts and deletes the namespace file at `path`, if present.
    fn release(path: &Path) -> io::Result<()> {
        sys::unmount(path)?;
        match std::fs::remove_file(path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

#[cfg(target_os = "linux")]
mod sys {
    use std::io;
    use std::path::Path;

    use rtnetlink::LinkUnspec;
    use rustix::mount::UnmountFlags;
    use rustix::thread::UnshareFlags;

    /// The loopback interface's index in every network namespace.
    const LOOPBACK: u32 = 1;

    /// Moves the calling thread into a new network namespace, binds it at `path` and brings
    /// loopback up. The thread must not run anything else afterwards.
    pub(super) fn create(path: &Path) -> io::Result<()> {
        // SAFETY: only the network namespace is unshared, never the file descriptor table, so
        // no thread can observe descriptors from another table.
        #[allow(
            unsafe_code,
            reason = "unshare has no safe form; see the safety comment"
        )]
        unsafe {
            rustix::thread::unshare_unsafe(UnshareFlags::NEWNET)?;
        }
        std::fs::File::create(path)?;
        rustix::mount::mount_bind("/proc/thread-self/ns/net", path)?;
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()?
            .block_on(loopback_up())
    }

    /// Brings loopback up in the calling thread's network namespace.
    async fn loopback_up() -> io::Result<()> {
        let (connection, handle, _) = rtnetlink::new_connection()?;
        let up = handle
            .link()
            .set(LinkUnspec::new_with_index(LOOPBACK).up().build())
            .execute();
        tokio::select! {
            result = up => result.map_err(io::Error::other),
            () = connection => Err(io::Error::other("the netlink connection closed")),
        }
    }

    /// Detaches the mount at `path`, if mounted.
    pub(super) fn unmount(path: &Path) -> io::Result<()> {
        match rustix::mount::unmount(path, UnmountFlags::DETACH) {
            Ok(()) | Err(rustix::io::Errno::INVAL | rustix::io::Errno::NOENT) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod sys {
    use std::io;
    use std::path::Path;

    pub(super) fn create(_: &Path) -> io::Result<()> {
        Err(io::Error::other("network namespaces need Linux"))
    }

    /// Nothing is ever mounted off Linux.
    #[allow(clippy::unnecessary_wraps, reason = "matches the Linux signature")]
    pub(super) fn unmount(_: &Path) -> io::Result<()> {
        Ok(())
    }
}
