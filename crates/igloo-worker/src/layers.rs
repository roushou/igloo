use std::collections::HashMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use igloo_core::Digest;
use igloo_core::snapshot::MediaType;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::blobs::{BlobSource, BlobSourceError, RemoteLayer};
use crate::rootfs::Rootfs;
use crate::store::LocalStore;

/// Snapshot layers, each downloaded, verified and unpacked once, then shared by every sandbox
/// that uses it:
///
/// ```text
/// layers/<hex>/        the unpacked layer, in the form the worker's Rootfs reads
/// layers/<hex>.json    its size and when it was last used
/// downloads/           archives being downloaded
/// ```
///
/// Invariant: a layer directory is complete once its metadata exists; partial work lives under
/// other names and is cleared on open. Pinned layers are never evicted.
#[derive(Clone)]
pub(crate) struct LayerCache {
    inner: Arc<Inner>,
}

struct Inner {
    source: Arc<dyn BlobSource>,
    layers: PathBuf,
    downloads: PathBuf,
    rootfs: Rootfs,
    budget: u64,
    index: Mutex<Index>,
    flights: Mutex<HashMap<Digest, Arc<tokio::sync::Mutex<()>>>>,
}

/// What is cached, what is in use, and the use counter that orders entries.
#[derive(Default)]
struct Index {
    entries: HashMap<Digest, Entry>,
    pins: HashMap<Digest, usize>,
    tick: u64,
}

/// One cached layer.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Entry {
    /// Bytes of file content.
    size: u64,
    /// The use counter at its last use; lower is older.
    used: u64,
}

/// Why a layer could not be cached.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LayerError {
    #[error(transparent)]
    Fetch(#[from] BlobSourceError),
    #[error("layer {0} does not match its digest")]
    Corrupt(Digest),
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl LayerCache {
    /// The cache under `data_dir`, keeping at most `budget` bytes of unpinned layers.
    pub(crate) async fn open(
        source: Arc<dyn BlobSource>,
        data_dir: &Path,
        rootfs: Rootfs,
        budget: u64,
    ) -> io::Result<Self> {
        let layers = data_dir.join("layers");
        let downloads = data_dir.join("downloads");
        tokio::fs::create_dir_all(&layers).await?;
        Self::remove_dir(&downloads).await?;
        tokio::fs::create_dir_all(&downloads).await?;
        let mut index = Index::default();
        let mut listing = tokio::fs::read_dir(&layers).await?;
        let mut stray = Vec::new();
        while let Some(item) = listing.next_entry().await? {
            let path = item.path();
            let name = item.file_name().to_string_lossy().into_owned();
            let entry = match name.strip_suffix(".json") {
                Some(hex) => Self::load_entry(&path, hex).await,
                None if Self::digest_of(&name).is_some() => continue,
                None => None,
            };
            match entry {
                Some((digest, entry)) => {
                    index.tick = index.tick.max(entry.used);
                    index.entries.insert(digest, entry);
                }
                None => stray.push(path),
            }
        }
        for path in stray {
            Self::remove_path(&path).await?;
        }
        let cache = Self {
            inner: Arc::new(Inner {
                source,
                layers,
                downloads,
                rootfs,
                budget,
                index: Mutex::new(index),
                flights: Mutex::new(HashMap::new()),
            }),
        };
        cache.remove_incomplete().await?;
        Ok(cache)
    }

    /// Keeps `layers` from eviction until a matching [`LayerCache::unpin`].
    pub(crate) fn pin(&self, layers: &[Digest]) {
        let mut index = self.index();
        for digest in layers {
            *index.pins.entry(*digest).or_default() += 1;
        }
    }

    /// Releases one [`LayerCache::pin`] of `layers`.
    pub(crate) fn unpin(&self, layers: &[Digest]) {
        let mut index = self.index();
        for digest in layers {
            if let Some(count) = index.pins.get_mut(digest) {
                *count -= 1;
                if *count == 0 {
                    index.pins.remove(digest);
                }
            }
        }
    }

    /// The unpacked directory of `layer`, downloading, verifying and unpacking it first if it
    /// is not cached. Concurrent calls for one digest download it once.
    pub(crate) async fn ensure(&self, layer: &RemoteLayer) -> Result<PathBuf, LayerError> {
        let digest = layer.layer().digest();
        let flight = self.flight(digest);
        let _guard = flight.lock().await;
        if self.index().entries.contains_key(&digest) {
            self.touch(digest).await?;
            return Ok(self.path(digest));
        }
        let archive = self.inner.downloads.join(Self::hex(digest));
        let mut file = tokio::fs::File::create(&archive).await?;
        let mut downloaded = self.inner.source.download(layer, &mut file).await;
        // Writes to a tokio file finish in the background; wait for them before reading back.
        if downloaded.is_ok()
            && let Err(error) = file.flush().await
        {
            downloaded = Err(BlobSourceError::Transport(Box::new(error)));
        }
        drop(file);
        let installed = match downloaded {
            Ok(()) => {
                self.install(&archive, digest, layer.layer().media_type())
                    .await
            }
            Err(error) => Err(error.into()),
        };
        Self::remove_path(&archive).await?;
        installed
    }

    /// The digest of the file at `archive`.
    pub(crate) async fn digest(archive: &Path) -> io::Result<Digest> {
        let file = archive.to_path_buf();
        tokio::task::spawn_blocking(move || -> io::Result<Digest> {
            let mut hasher = blake3::Hasher::new();
            hasher.update_reader(std::fs::File::open(file)?)?;
            Ok(Digest::from_blake3(*hasher.finalize().as_bytes()))
        })
        .await
        .map_err(io::Error::other)?
    }

    /// Caches the tar archive at `archive`, whose digest is `digest`, such as a layer this
    /// worker just sealed, so forks placed here need not download it. The archive is left in
    /// place.
    pub(crate) async fn adopt(&self, archive: &Path, digest: Digest) -> Result<(), LayerError> {
        let flight = self.flight(digest);
        let _guard = flight.lock().await;
        if !self.index().entries.contains_key(&digest) {
            self.install(archive, digest, MediaType::Tar).await?;
        }
        Ok(())
    }

    /// Verifies `archive` against `digest`, unpacks it and records it. The caller holds the
    /// digest's flight lock.
    async fn install(
        &self,
        archive: &Path,
        digest: Digest,
        media_type: MediaType,
    ) -> Result<PathBuf, LayerError> {
        let path = self.path(digest);
        let partial = self
            .inner
            .layers
            .join(format!("{}.partial", Self::hex(digest)));
        let rootfs = self.inner.rootfs;
        let (source, target) = (archive.to_path_buf(), partial.clone());
        let unpacked = tokio::task::spawn_blocking(move || {
            Self::verify_and_unpack(&source, digest, media_type, rootfs, &target)
        })
        .await
        .map_err(io::Error::other)
        .map_err(LayerError::from)
        .and_then(|result| result);
        let size = match unpacked {
            Ok(size) => size,
            Err(error) => {
                Self::remove_path(&partial).await?;
                return Err(error);
            }
        };
        Self::remove_path(&path).await?;
        tokio::fs::rename(&partial, &path).await?;
        self.index().entries.insert(digest, Entry { size, used: 0 });
        self.touch(digest).await?;
        Ok(path)
    }

    /// Removes the least recently used unpinned layers until the unpinned ones fit the budget.
    pub(crate) async fn evict(&self) -> io::Result<()> {
        let victims = {
            let index = self.index();
            let mut unpinned: Vec<(Digest, Entry)> = index
                .entries
                .iter()
                .filter(|(digest, _)| !index.pins.contains_key(digest))
                .map(|(digest, entry)| (*digest, *entry))
                .collect();
            unpinned.sort_by_key(|(_, entry)| entry.used);
            let mut total: u64 = unpinned.iter().map(|(_, entry)| entry.size).sum();
            let mut victims = Vec::new();
            for (digest, entry) in unpinned {
                if total <= self.inner.budget {
                    break;
                }
                total -= entry.size;
                victims.push(digest);
            }
            victims
        };
        for digest in victims {
            let flight = self.flight(digest);
            let _guard = flight.lock().await;
            if self.index().pins.contains_key(&digest) {
                continue;
            }
            self.index().entries.remove(&digest);
            Self::remove_path(&self.meta_path(digest)).await?;
            Self::remove_path(&self.path(digest)).await?;
        }
        Ok(())
    }

    /// Bytes of file content the cache holds, pinned layers included.
    pub(crate) fn size(&self) -> u64 {
        self.index().entries.values().map(|entry| entry.size).sum()
    }

    /// The budget above which unpinned layers are evicted.
    pub(crate) fn budget(&self) -> u64 {
        self.inner.budget
    }

    /// Whether `digest` is cached.
    #[cfg(test)]
    pub(crate) fn contains(&self, digest: Digest) -> bool {
        self.index().entries.contains_key(&digest)
    }

    fn path(&self, digest: Digest) -> PathBuf {
        self.inner.layers.join(Self::hex(digest))
    }

    fn meta_path(&self, digest: Digest) -> PathBuf {
        self.inner
            .layers
            .join(format!("{}.json", Self::hex(digest)))
    }

    fn index(&self) -> std::sync::MutexGuard<'_, Index> {
        self.inner
            .index
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn flight(&self, digest: Digest) -> Arc<tokio::sync::Mutex<()>> {
        let mut flights = self
            .inner
            .flights
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Arc::clone(flights.entry(digest).or_default())
    }

    /// Records a use of `digest` and persists its entry.
    async fn touch(&self, digest: Digest) -> io::Result<()> {
        let entry = {
            let mut index = self.index();
            index.tick += 1;
            let tick = index.tick;
            let Some(entry) = index.entries.get_mut(&digest) else {
                return Ok(());
            };
            entry.used = tick;
            *entry
        };
        let bytes = serde_json::to_vec(&entry).map_err(io::Error::other)?;
        LocalStore::write_atomically(&self.meta_path(digest), &bytes).await
    }

    /// Checks the archive against `digest`, then unpacks it into `target`; returns its size.
    fn verify_and_unpack(
        archive: &Path,
        digest: Digest,
        media_type: MediaType,
        rootfs: Rootfs,
        target: &Path,
    ) -> Result<u64, LayerError> {
        let mut hasher = blake3::Hasher::new();
        hasher.update_reader(std::fs::File::open(archive)?)?;
        if Digest::from_blake3(*hasher.finalize().as_bytes()) != digest {
            return Err(LayerError::Corrupt(digest));
        }
        if target.exists() {
            std::fs::remove_dir_all(target)?;
        }
        let file = std::fs::File::open(archive)?;
        let reader: Box<dyn Read> = match media_type {
            MediaType::Tar => Box::new(file),
            MediaType::TarGzip => Box::new(flate2::read::GzDecoder::new(file)),
        };
        Ok(rootfs.unpack(reader, target)?)
    }

    /// Drops index entries whose directory is missing, and directories without an entry.
    async fn remove_incomplete(&self) -> io::Result<()> {
        let digests: Vec<Digest> = self.index().entries.keys().copied().collect();
        for digest in digests {
            if !tokio::fs::try_exists(self.path(digest)).await? {
                self.index().entries.remove(&digest);
                Self::remove_path(&self.meta_path(digest)).await?;
            }
        }
        let mut listing = tokio::fs::read_dir(&self.inner.layers).await?;
        let mut orphans = Vec::new();
        while let Some(item) = listing.next_entry().await? {
            let name = item.file_name().to_string_lossy().into_owned();
            if let Some(digest) = Self::digest_of(&name)
                && !self.index().entries.contains_key(&digest)
            {
                orphans.push(item.path());
            }
        }
        for path in orphans {
            Self::remove_path(&path).await?;
        }
        Ok(())
    }

    async fn load_entry(path: &Path, hex: &str) -> Option<(Digest, Entry)> {
        let digest = Self::digest_of(hex)?;
        let bytes = tokio::fs::read(path).await.ok()?;
        let entry = serde_json::from_slice(&bytes).ok()?;
        Some((digest, entry))
    }

    fn digest_of(hex: &str) -> Option<Digest> {
        format!("blake3:{hex}").parse().ok()
    }

    fn hex(digest: Digest) -> String {
        let text = digest.to_string();
        text.strip_prefix("blake3:").unwrap_or(&text).to_owned()
    }

    async fn remove_path(path: &Path) -> io::Result<()> {
        match tokio::fs::symlink_metadata(path).await {
            Ok(metadata) if metadata.is_dir() => Self::remove_dir(path).await,
            Ok(_) => tokio::fs::remove_file(path).await,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    async fn remove_dir(path: &Path) -> io::Result<()> {
        match tokio::fs::remove_dir_all(path).await {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use igloo_core::snapshot::SnapshotLayer;
    use tokio::io::AsyncWriteExt;

    use super::*;

    /// Serves one-file layers by digest, counting downloads.
    #[derive(Default)]
    struct Layers {
        archives: Mutex<HashMap<Digest, Vec<u8>>>,
        downloads: AtomicUsize,
    }

    impl Layers {
        /// A layer holding `name` with `size` bytes.
        fn add(&self, name: &str, size: usize) -> RemoteLayer {
            let mut builder = tar::Builder::new(Vec::new());
            let mut header = tar::Header::new_gnu();
            header.set_size(size as u64);
            header.set_mode(0o644);
            header.set_uid(0);
            header.set_gid(0);
            header.set_cksum();
            builder
                .append_data(&mut header, name, vec![b'x'; size].as_slice())
                .expect("append");
            let bytes = builder.into_inner().expect("tar");
            let digest = Digest::from_blake3(*blake3::hash(&bytes).as_bytes());
            self.archives
                .lock()
                .expect("archives")
                .insert(digest, bytes);
            RemoteLayer::new(
                SnapshotLayer::new(digest, MediaType::Tar),
                format!("http://blobs.test/{digest}").parse().expect("url"),
            )
        }
    }

    #[async_trait]
    impl BlobSource for Layers {
        async fn download(
            &self,
            layer: &RemoteLayer,
            file: &mut tokio::fs::File,
        ) -> Result<(), BlobSourceError> {
            self.downloads.fetch_add(1, Ordering::Relaxed);
            let digest = layer.layer().digest();
            let bytes = self
                .archives
                .lock()
                .expect("archives")
                .get(&digest)
                .cloned()
                .ok_or(BlobSourceError::NotFound(digest))?;
            file.write_all(&bytes)
                .await
                .map_err(|error| BlobSourceError::Transport(Box::new(error)))?;
            file.flush()
                .await
                .map_err(|error| BlobSourceError::Transport(Box::new(error)))
        }

        async fn upload(
            &self,
            _: &reqwest::Url,
            _: tokio::fs::File,
        ) -> Result<(), BlobSourceError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn layers_are_downloaded_once_and_survive_a_reopen() {
        let dir = tempfile::tempdir().expect("dir");
        let source = Arc::new(Layers::default());
        let layer = source.add("a", 10);
        let cache = LayerCache::open(source.clone(), dir.path(), Rootfs::Copy, 1 << 20)
            .await
            .expect("open");
        let (first, second) = tokio::join!(cache.ensure(&layer), cache.ensure(&layer));
        assert_eq!(first.expect("first"), second.expect("second"));
        assert_eq!(source.downloads.load(Ordering::Relaxed), 1);

        let reopened = LayerCache::open(source.clone(), dir.path(), Rootfs::Copy, 1 << 20)
            .await
            .expect("reopen");
        let path = reopened.ensure(&layer).await.expect("cached");
        assert!(path.join("a").exists());
        assert_eq!(source.downloads.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn the_least_recently_used_unpinned_layers_are_evicted() {
        let dir = tempfile::tempdir().expect("dir");
        let source = Arc::new(Layers::default());
        let (a, b, c) = (
            source.add("a", 100),
            source.add("b", 100),
            source.add("c", 100),
        );
        let cache = LayerCache::open(source, dir.path(), Rootfs::Copy, 150)
            .await
            .expect("open");
        for layer in [&a, &b, &c] {
            cache.ensure(layer).await.expect("ensure");
        }
        cache.pin(&[a.layer().digest()]);
        cache.ensure(&b).await.expect("use b again");
        cache.evict().await.expect("evict");
        assert!(cache.contains(a.layer().digest()), "pinned layers stay");
        assert!(
            cache.contains(b.layer().digest()),
            "the most recent layer stays"
        );
        assert!(!cache.contains(c.layer().digest()));
        assert!(
            !dir.path()
                .join("layers")
                .join(LayerCache::hex(c.layer().digest()))
                .exists()
        );

        cache.unpin(&[a.layer().digest()]);
        cache.evict().await.expect("evict");
        assert!(!cache.contains(a.layer().digest()));
        assert!(cache.contains(b.layer().digest()));
    }

    #[tokio::test]
    async fn a_corrupt_download_is_refused_and_leaves_nothing() {
        let dir = tempfile::tempdir().expect("dir");
        let source = Arc::new(Layers::default());
        let layer = source.add("a", 10);
        let digest = layer.layer().digest();
        source
            .archives
            .lock()
            .expect("archives")
            .insert(digest, b"tampered".to_vec());
        let cache = LayerCache::open(source, dir.path(), Rootfs::Copy, 1 << 20)
            .await
            .expect("open");
        assert!(matches!(
            cache.ensure(&layer).await,
            Err(LayerError::Corrupt(_))
        ));
        assert!(!cache.contains(digest));
        let leftovers = std::fs::read_dir(dir.path().join("layers"))
            .expect("layers")
            .count()
            + std::fs::read_dir(dir.path().join("downloads"))
                .expect("downloads")
                .count();
        assert_eq!(leftovers, 0);
    }
}
