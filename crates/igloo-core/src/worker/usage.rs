/// What a worker holds of its machine at one moment. Sizes are bytes.
///
/// Invariant: a value is a report, not a promise; nothing in the domain depends on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    disk_total_bytes: u64,
    disk_free_bytes: u64,
    layer_cache_bytes: u64,
    layer_cache_limit_bytes: u64,
    sandboxes: u32,
}

impl Usage {
    /// A usage report. `disk_*` describe the file system holding the worker's data directory,
    /// `layer_cache_*` the layer cache and its budget, `sandboxes` the sandboxes held.
    #[must_use]
    pub const fn new(
        disk_total_bytes: u64,
        disk_free_bytes: u64,
        layer_cache_bytes: u64,
        layer_cache_limit_bytes: u64,
        sandboxes: u32,
    ) -> Self {
        Self {
            disk_total_bytes,
            disk_free_bytes,
            layer_cache_bytes,
            layer_cache_limit_bytes,
            sandboxes,
        }
    }

    /// Total space of the data directory's file system.
    #[must_use]
    pub const fn disk_total_bytes(&self) -> u64 {
        self.disk_total_bytes
    }

    /// Free space of the data directory's file system.
    #[must_use]
    pub const fn disk_free_bytes(&self) -> u64 {
        self.disk_free_bytes
    }

    /// Bytes the layer cache holds.
    #[must_use]
    pub const fn layer_cache_bytes(&self) -> u64 {
        self.layer_cache_bytes
    }

    /// The budget above which unpinned layers are evicted.
    #[must_use]
    pub const fn layer_cache_limit_bytes(&self) -> u64 {
        self.layer_cache_limit_bytes
    }

    /// Sandboxes held, starting or ready.
    #[must_use]
    pub const fn sandboxes(&self) -> u32 {
        self.sandboxes
    }
}
