use std::io;
use std::path::PathBuf;

use igloo_core::worker::Usage;

use crate::layers::LayerCache;

/// Measures what the worker holds of its machine.
///
/// Invariant: a measurement is a snapshot of the moment it is taken; it never fails the
/// worker, so a disk that cannot be read reports zero bytes.
pub(crate) struct UsageMeter {
    data_dir: PathBuf,
    layers: LayerCache,
}

impl UsageMeter {
    /// A meter of the file system holding `data_dir` and of `layers`.
    pub(crate) const fn new(data_dir: PathBuf, layers: LayerCache) -> Self {
        Self { data_dir, layers }
    }

    /// The usage now, with `sandboxes` held.
    pub(crate) fn measure(&self, sandboxes: usize) -> Usage {
        let (total, free) = self.disk().unwrap_or_else(|error| {
            tracing::warn!(%error, "cannot read the data directory's file system");
            (0, 0)
        });
        Usage::new(
            total,
            free,
            self.layers.size(),
            self.layers.budget(),
            u32::try_from(sandboxes).unwrap_or(u32::MAX),
        )
    }

    /// Total and free bytes of the data directory's file system; free is what unprivileged
    /// processes may still use.
    #[cfg(unix)]
    fn disk(&self) -> io::Result<(u64, u64)> {
        let stats = rustix::fs::statvfs(&self.data_dir)?;
        Ok((
            stats.f_blocks.saturating_mul(stats.f_frsize),
            stats.f_bavail.saturating_mul(stats.f_frsize),
        ))
    }

    #[cfg(not(unix))]
    fn disk(&self) -> io::Result<(u64, u64)> {
        let _ = &self.data_dir;
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}
