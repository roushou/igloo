use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// How sandbox file systems are assembled from cached layers.
///
/// Layers are cached in the form the strategy reads: [`Rootfs::Copy`] keeps OCI whiteout
/// markers (`.wh.<name>`, `.wh..wh..opq`) as files and applies them while copying;
/// [`Rootfs::Overlay`] turns them into overlayfs whiteouts (0/0 character devices and the
/// `trusted.overlay.opaque` attribute) and mounts the layers read-only under a private upper
/// directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rootfs {
    /// Copies every layer into the sandbox. Works anywhere; costs a copy per sandbox.
    Copy,
    /// Mounts an overlay of the cached layers. Linux only, and needs `CAP_SYS_ADMIN`.
    Overlay,
}

/// What a sealed layer holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Packed {
    /// The whole root file system; it replaces the snapshot's layers.
    Full,
    /// The changes over the snapshot's layers, deletions as whiteouts.
    Changes,
}

/// Writes a directory tree as a tar layer, never following symbolic links.
struct LayerPack<W: Write> {
    builder: tar::Builder<W>,
    /// Whether the tree is an overlay upper directory, whose whiteouts become OCI markers.
    upper: bool,
}

/// Applies one layer directory, in copy form, over a sandbox root.
struct LayerCopy<'a> {
    target: &'a Path,
}

impl Rootfs {
    const WHITEOUT: &'static str = ".wh.";
    const OPAQUE: &'static str = ".wh..wh..opq";

    /// Unpacks a layer archive into the empty directory `target`, in this strategy's form.
    /// Returns the bytes of file content written.
    pub(crate) fn unpack(self, reader: impl Read, target: &Path) -> io::Result<u64> {
        std::fs::create_dir_all(target)?;
        let mut archive = tar::Archive::new(reader);
        archive.set_preserve_permissions(true);
        archive.set_preserve_ownerships(self == Self::Overlay);
        archive.set_overwrite(true);
        let mut bytes = 0;
        for entry in archive.entries()? {
            let mut entry = entry?;
            bytes += entry.header().size()?;
            if self == Self::Overlay {
                let path = entry.path()?.into_owned();
                if Self::overlay_whiteout(target, &path)? {
                    continue;
                }
            }
            entry.unpack_in(target)?;
        }
        Ok(bytes)
    }

    /// Builds the root of the sandbox in `sandbox_dir` from `layers`, lowest first; returns it.
    pub(crate) fn assemble(self, layers: &[PathBuf], sandbox_dir: &Path) -> io::Result<PathBuf> {
        let root = sandbox_dir.join("rootfs");
        std::fs::create_dir_all(&root)?;
        match self {
            Self::Copy => {
                for layer in layers {
                    LayerCopy { target: &root }.apply(layer)?;
                }
            }
            Self::Overlay => overlay::mount(layers, sandbox_dir, &root)?,
        }
        Ok(root)
    }

    /// Undoes [`Rootfs::assemble`] short of deleting files: unmounts an overlay root.
    pub(crate) fn release(self, sandbox_dir: &Path) -> io::Result<()> {
        match self {
            Self::Copy => Ok(()),
            Self::Overlay => overlay::unmount(&sandbox_dir.join("rootfs")),
        }
    }

    /// Packs the sandbox in `sandbox_dir` as a tar layer into `out`: with copies the whole
    /// root, with overlays the private upper directory, its whiteouts written as OCI markers.
    /// File times, modes and owners are kept.
    pub(crate) fn pack(self, sandbox_dir: &Path, out: impl Write) -> io::Result<Packed> {
        let (tree, upper, packed) = match self {
            Self::Copy => (sandbox_dir.join("rootfs"), false, Packed::Full),
            Self::Overlay => (sandbox_dir.join("upper"), true, Packed::Changes),
        };
        let mut builder = tar::Builder::new(out);
        builder.follow_symlinks(false);
        let mut pack = LayerPack { builder, upper };
        pack.add_dir(&tree, Path::new(""))?;
        pack.builder.into_inner()?.flush()?;
        Ok(packed)
    }

    /// Writes the overlayfs form of `path` if it is a whiteout marker; reports whether it was.
    fn overlay_whiteout(target: &Path, path: &Path) -> io::Result<bool> {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return Ok(false);
        };
        let parent = target.join(path.parent().unwrap_or_else(|| Path::new("")));
        if !LayerCopy::is_inside(path) {
            return Err(io::Error::other(format!(
                "{} escapes the layer",
                path.display()
            )));
        }
        if name == Self::OPAQUE {
            std::fs::create_dir_all(&parent)?;
            overlay::mark_opaque(&parent)?;
            return Ok(true);
        }
        if let Some(hidden) = name.strip_prefix(Self::WHITEOUT) {
            std::fs::create_dir_all(&parent)?;
            overlay::whiteout(&parent.join(hidden))?;
            return Ok(true);
        }
        Ok(false)
    }
}

impl<W: Write> LayerPack<W> {
    fn add_dir(&mut self, tree: &Path, relative: &Path) -> io::Result<()> {
        let dir = tree.join(relative);
        let mut names = std::fs::read_dir(&dir)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        names.sort();
        if self.upper && !relative.as_os_str().is_empty() && overlay::is_opaque(&dir)? {
            self.marker(&relative.join(Rootfs::OPAQUE))?;
        }
        for name in names {
            let path = dir.join(&name);
            let entry = relative.join(&name);
            let metadata = std::fs::symlink_metadata(&path)?;
            let kind = metadata.file_type();
            if self.upper && Self::is_whiteout(&metadata) {
                let mut marker = std::ffi::OsString::from(Rootfs::WHITEOUT);
                marker.push(&name);
                self.marker(&relative.join(marker))?;
            } else if kind.is_dir() {
                self.builder.append_path_with_name(&path, &entry)?;
                self.add_dir(tree, &entry)?;
            } else if kind.is_file() || kind.is_symlink() {
                self.builder.append_path_with_name(&path, &entry)?;
            }
        }
        Ok(())
    }

    /// Appends an empty whiteout marker file.
    fn marker(&mut self, path: &Path) -> io::Result<()> {
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o600);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_entry_type(tar::EntryType::Regular);
        self.builder.append_data(&mut header, path, io::empty())
    }

    /// Whether `metadata` describes an overlayfs whiteout: a 0/0 character device.
    #[cfg(unix)]
    fn is_whiteout(metadata: &std::fs::Metadata) -> bool {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        metadata.file_type().is_char_device() && metadata.rdev() == 0
    }

    #[cfg(not(unix))]
    fn is_whiteout(_: &std::fs::Metadata) -> bool {
        false
    }
}

impl LayerCopy<'_> {
    /// Copies `layer` over the target: an opaque marker empties its directory first, a
    /// `.wh.<name>` marker deletes `<name>`, and an entry replaces whatever lower entry of
    /// another kind it lands on. Symbolic links in the target are never followed.
    fn apply(&self, layer: &Path) -> io::Result<()> {
        self.apply_dir(layer, Path::new(""))
    }

    fn apply_dir(&self, layer: &Path, relative: &Path) -> io::Result<()> {
        let source = layer.join(relative);
        let target = self.target.join(relative);
        let mut entries = std::fs::read_dir(&source)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        entries.sort();
        if entries.iter().any(|name| name == Rootfs::OPAQUE) {
            Self::clear(&target)?;
        }
        for name in entries {
            let Some(text) = name.to_str() else {
                continue;
            };
            if text == Rootfs::OPAQUE {
                continue;
            }
            if let Some(hidden) = text.strip_prefix(Rootfs::WHITEOUT) {
                Self::remove(&target.join(hidden))?;
                continue;
            }
            let from = source.join(&name);
            let to = target.join(&name);
            let kind = std::fs::symlink_metadata(&from)?.file_type();
            let existing = std::fs::symlink_metadata(&to)
                .ok()
                .map(|meta| meta.file_type());
            if kind.is_dir() {
                if !existing.is_some_and(|existing| existing.is_dir()) {
                    Self::remove(&to)?;
                    std::fs::create_dir(&to)?;
                }
                std::fs::set_permissions(&to, std::fs::metadata(&from)?.permissions())?;
                self.apply_dir(layer, &relative.join(&name))?;
            } else {
                Self::remove(&to)?;
                if kind.is_symlink() {
                    Self::symlink(&std::fs::read_link(&from)?, &to)?;
                } else {
                    std::fs::copy(&from, &to)?;
                }
            }
        }
        Ok(())
    }

    /// Whether `relative` stays inside the directory it is relative to.
    fn is_inside(relative: &Path) -> bool {
        relative.components().all(|component| {
            matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    }

    fn remove(path: &Path) -> io::Result<()> {
        let removed = match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
            Ok(_) => std::fs::remove_file(path),
            Err(error) => Err(error),
        };
        match removed {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    fn clear(dir: &Path) -> io::Result<()> {
        if !std::fs::symlink_metadata(dir).is_ok_and(|metadata| metadata.is_dir()) {
            return Ok(());
        }
        for entry in std::fs::read_dir(dir)? {
            Self::remove(&entry?.path())?;
        }
        Ok(())
    }

    #[cfg(unix)]
    fn symlink(original: &Path, link: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(original, link)
    }

    #[cfg(not(unix))]
    fn symlink(_: &Path, _: &Path) -> io::Result<()> {
        Err(io::Error::other("symbolic links need a Unix host"))
    }
}

#[cfg(target_os = "linux")]
mod overlay {
    use std::io;
    use std::path::{Path, PathBuf};

    use rustix::fs::{CWD, FileType, Mode, XattrFlags, getxattr, makedev, mknodat, setxattr};
    use rustix::mount::{self as sys, MountFlags, UnmountFlags};

    /// Mounts `layers` (lowest first) read-only under `sandbox_dir/upper` at `root`.
    pub(super) fn mount(layers: &[PathBuf], sandbox_dir: &Path, root: &Path) -> io::Result<()> {
        let upper = sandbox_dir.join("upper");
        let work = sandbox_dir.join("work");
        std::fs::create_dir_all(&upper)?;
        std::fs::create_dir_all(&work)?;
        let lower: Vec<String> = layers
            .iter()
            .rev()
            .map(|layer| layer.display().to_string())
            .collect();
        let options = format!(
            "lowerdir={},upperdir={},workdir={}",
            lower.join(":"),
            upper.display(),
            work.display()
        );
        let options = std::ffi::CString::new(options).map_err(io::Error::other)?;
        sys::mount(
            "overlay",
            root,
            "overlay",
            MountFlags::empty(),
            options.as_c_str(),
        )?;
        Ok(())
    }

    /// Detaches the overlay at `root`, if mounted.
    pub(super) fn unmount(root: &Path) -> io::Result<()> {
        match sys::unmount(root, UnmountFlags::DETACH) {
            Ok(()) | Err(rustix::io::Errno::INVAL | rustix::io::Errno::NOENT) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Writes an overlayfs whiteout at `path`: a 0/0 character device.
    pub(super) fn whiteout(path: &Path) -> io::Result<()> {
        mknodat(
            CWD,
            path,
            FileType::CharacterDevice,
            Mode::empty(),
            makedev(0, 0),
        )?;
        Ok(())
    }

    /// Marks `dir` opaque: lower layers' entries under it are hidden.
    pub(super) fn mark_opaque(dir: &Path) -> io::Result<()> {
        setxattr(dir, OPAQUE, b"y", XattrFlags::empty())?;
        Ok(())
    }

    /// Whether `dir` is marked opaque.
    pub(super) fn is_opaque(dir: &Path) -> io::Result<bool> {
        let mut value = [0u8; 1];
        match getxattr(dir, OPAQUE, &mut value) {
            Ok(read) => Ok(read == 1 && value == *b"y"),
            Err(rustix::io::Errno::NODATA) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    const OPAQUE: &str = "trusted.overlay.opaque";
}

#[cfg(not(target_os = "linux"))]
mod overlay {
    use std::io;
    use std::path::{Path, PathBuf};

    fn unsupported() -> io::Error {
        io::Error::other("overlay file systems need Linux")
    }

    pub(super) fn mount(_: &[PathBuf], _: &Path, _: &Path) -> io::Result<()> {
        Err(unsupported())
    }

    pub(super) fn unmount(_: &Path) -> io::Result<()> {
        Err(unsupported())
    }

    pub(super) fn whiteout(_: &Path) -> io::Result<()> {
        Err(unsupported())
    }

    pub(super) fn mark_opaque(_: &Path) -> io::Result<()> {
        Err(unsupported())
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "mirrors the Linux signature, which reads an attribute"
    )]
    pub(super) fn is_opaque(_: &Path) -> io::Result<bool> {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, contents) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(0);
            header.set_cksum();
            builder
                .append_data(&mut header, path, *contents)
                .expect("append");
        }
        builder.into_inner().expect("tar")
    }

    /// Unpacks a lower and an upper layer with whiteouts, assembles them and checks the view.
    fn whiteouts_apply(rootfs: Rootfs) {
        let dir = tempfile::tempdir().expect("dir");
        let lower = archive(&[
            ("etc/keep", b"1"),
            ("etc/drop", b"2"),
            ("cache/a", b"3"),
            ("cache/b", b"4"),
        ]);
        let upper = archive(&[
            ("etc/.wh.drop", b""),
            ("cache/.wh..wh..opq", b""),
            ("cache/c", b"5"),
        ]);
        let layers = [dir.path().join("lower"), dir.path().join("upper")];
        assert_eq!(
            rootfs.unpack(lower.as_slice(), &layers[0]).expect("lower"),
            4
        );
        rootfs.unpack(upper.as_slice(), &layers[1]).expect("upper");
        let sandbox = dir.path().join("sandbox");
        let root = rootfs.assemble(&layers, &sandbox).expect("assemble");
        assert!(root.join("etc/keep").exists());
        assert!(!root.join("etc/drop").exists());
        assert!(!root.join("cache/a").exists());
        assert!(root.join("cache/c").exists());

        std::fs::write(root.join("etc/keep"), "changed").expect("write");
        assert_eq!(
            std::fs::read(layers[0].join("etc/keep")).expect("cached"),
            b"1",
            "writes stay in the sandbox"
        );
        rootfs.release(&sandbox).expect("release");
    }

    /// Seals a sandbox built from one base layer after changing it, then forks: the fork sees
    /// the sealed files and deletions.
    fn a_fork_of_a_seal_sees_its_changes(rootfs: Rootfs) {
        let dir = tempfile::tempdir().expect("dir");
        let base = dir.path().join("base");
        rootfs
            .unpack(
                archive(&[("keep", b"1"), ("drop", b"2"), ("cache/a", b"3")]).as_slice(),
                &base,
            )
            .expect("base");
        let parent = dir.path().join("parent");
        let root = rootfs
            .assemble(std::slice::from_ref(&base), &parent)
            .expect("assemble");
        std::fs::write(root.join("new"), "4").expect("write");
        std::fs::remove_file(root.join("drop")).expect("delete");
        std::fs::remove_dir_all(root.join("cache")).expect("delete dir");
        std::fs::create_dir(root.join("cache")).expect("recreate dir");
        std::fs::write(root.join("cache/b"), "5").expect("write");

        let mut tar = Vec::new();
        let packed = rootfs.pack(&parent, &mut tar).expect("pack");
        rootfs.release(&parent).expect("release");
        let sealed = dir.path().join("sealed");
        rootfs
            .unpack(tar.as_slice(), &sealed)
            .expect("sealed layer");
        let layers = match packed {
            Packed::Full => vec![sealed],
            Packed::Changes => vec![base, sealed],
        };
        let fork = dir.path().join("fork");
        let root = rootfs.assemble(&layers, &fork).expect("fork");
        assert_eq!(std::fs::read(root.join("keep")).expect("keep"), b"1");
        assert_eq!(std::fs::read(root.join("new")).expect("new"), b"4");
        assert!(!root.join("drop").exists(), "deletions are sealed");
        assert!(
            !root.join("cache/a").exists(),
            "replaced directories are sealed"
        );
        assert_eq!(std::fs::read(root.join("cache/b")).expect("b"), b"5");
        rootfs.release(&fork).expect("release");
    }

    #[test]
    fn a_fork_of_a_copied_seal_sees_its_changes() {
        a_fork_of_a_seal_sees_its_changes(Rootfs::Copy);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_fork_of_an_overlay_seal_sees_its_changes() {
        if std::env::var_os("IGLOO_TEST_OVERLAY").is_none() {
            return;
        }
        a_fork_of_a_seal_sees_its_changes(Rootfs::Overlay);
    }

    #[test]
    fn copies_apply_whiteouts_and_keep_the_cache_pristine() {
        whiteouts_apply(Rootfs::Copy);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn overlays_apply_whiteouts_and_keep_the_cache_pristine() {
        if std::env::var_os("IGLOO_TEST_OVERLAY").is_none() {
            return;
        }
        whiteouts_apply(Rootfs::Overlay);
    }

    #[test]
    fn copies_replace_lower_symlinks_instead_of_following_them() {
        let dir = tempfile::tempdir().expect("dir");
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).expect("outside");
        let lower = dir.path().join("lower");
        std::fs::create_dir(&lower).expect("lower");
        std::os::unix::fs::symlink(&outside, lower.join("etc")).expect("symlink");
        let upper = dir.path().join("upper");
        Rootfs::Copy
            .unpack(archive(&[("etc/passwd", b"x")]).as_slice(), &upper)
            .expect("upper");
        let root = Rootfs::Copy
            .assemble(&[lower, upper], &dir.path().join("sandbox"))
            .expect("assemble");
        assert!(root.join("etc/passwd").exists());
        assert!(!outside.join("passwd").exists());
    }

    #[test]
    fn overlay_whiteouts_cannot_escape_the_layer() {
        let dir = tempfile::tempdir().expect("dir");
        assert!(Rootfs::overlay_whiteout(dir.path(), Path::new("../.wh.outside")).is_err());
    }
}
