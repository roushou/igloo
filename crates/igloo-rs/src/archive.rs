use std::io;
use std::path::Path;

/// A directory packed as a snapshot layer: a tar archive of the files `.gitignore` rules keep,
/// rooted at [`Layer::ROOT`], with fixed timestamps and owners so the same tree always has the
/// same digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    bytes: Vec<u8>,
}

impl Layer {
    /// The directory the files are placed under: jobs run there.
    pub const ROOT: &'static str = "workspace";

    /// Packs `dir`, skipping `.git` and everything ignored by `.gitignore` and `.ignore` files.
    /// Reads the file system synchronously.
    pub fn from_dir(dir: &Path) -> io::Result<Self> {
        let mut entries: Vec<_> = ignore::WalkBuilder::new(dir)
            .hidden(false)
            .require_git(false)
            .filter_entry(|entry| entry.file_name() != ".git")
            .build()
            .filter_map(Result::ok)
            .filter(|entry| entry.depth() > 0)
            .collect();
        entries.sort_by(|a, b| a.path().cmp(b.path()));
        let mut archive = tar::Builder::new(Vec::new());
        archive.follow_symlinks(false);
        for entry in entries {
            let Some(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                continue;
            }
            let relative = Path::new(Self::ROOT)
                .join(entry.path().strip_prefix(dir).map_err(io::Error::other)?);
            let metadata = std::fs::symlink_metadata(entry.path())?;
            let mut header = tar::Header::new_gnu();
            header.set_metadata_in_mode(&metadata, tar::HeaderMode::Deterministic);
            if kind.is_symlink() {
                let target = std::fs::read_link(entry.path())?;
                archive.append_link(&mut header, &relative, target)?;
            } else {
                let file = std::fs::File::open(entry.path())?;
                archive.append_data(&mut header, &relative, file)?;
            }
        }
        Ok(Self {
            bytes: archive.into_inner()?,
        })
    }

    /// The archive bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The archive bytes, consumed.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_what_gitignore_keeps_deterministically() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join(".gitignore"), "target/\nsecret.txt\n").expect("write");
        std::fs::write(dir.path().join("hello.txt"), "hi\n").expect("write");
        std::fs::write(dir.path().join("secret.txt"), "no\n").expect("write");
        std::fs::create_dir_all(dir.path().join("target")).expect("mkdir");
        std::fs::write(dir.path().join("target/out"), "big").expect("write");
        std::fs::create_dir_all(dir.path().join(".git")).expect("mkdir");
        std::fs::write(dir.path().join(".git/HEAD"), "ref").expect("write");

        let layer = Layer::from_dir(dir.path()).expect("pack");
        let mut names: Vec<String> = tar::Archive::new(layer.as_bytes())
            .entries()
            .expect("entries")
            .map(|entry| {
                entry
                    .expect("entry")
                    .path()
                    .expect("path")
                    .display()
                    .to_string()
            })
            .collect();
        names.sort();
        assert_eq!(names, ["workspace/.gitignore", "workspace/hello.txt"]);
        assert_eq!(Layer::from_dir(dir.path()).expect("pack again"), layer);
    }
}
