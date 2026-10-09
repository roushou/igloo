use crate::path::RepoPath;

/// One file that differs between two commits, with the unified patch git prints for it.
///
/// Invariants: `previous` is set exactly when the status is [`FileStatus::Renamed`]; a binary
/// file has no counts and an empty patch; every other patch starts at its `diff --git` header,
/// exactly as git prints it (non-UTF-8 bytes become U+FFFD).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiff {
    status: FileStatus,
    path: RepoPath,
    previous: Option<RepoPath>,
    additions: u64,
    deletions: u64,
    binary: bool,
    patch: String,
}

/// How a file differs, with renames detected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    /// Absent before, present after.
    Added,
    /// Present in both with different content, mode or kind.
    Modified,
    /// Present before, absent after.
    Deleted,
    /// Moved from another path, possibly with changes.
    Renamed,
}

impl FileDiff {
    /// How the file differs.
    #[must_use]
    pub const fn status(&self) -> FileStatus {
        self.status
    }

    /// The file's path after the change; for a deletion, the path it had.
    #[must_use]
    pub const fn path(&self) -> &RepoPath {
        &self.path
    }

    /// The path a renamed file had before.
    #[must_use]
    pub const fn previous(&self) -> Option<&RepoPath> {
        self.previous.as_ref()
    }

    /// Lines added; zero for a binary file.
    #[must_use]
    pub const fn additions(&self) -> u64 {
        self.additions
    }

    /// Lines removed; zero for a binary file.
    #[must_use]
    pub const fn deletions(&self) -> u64 {
        self.deletions
    }

    /// Whether git treats the file as binary.
    #[must_use]
    pub const fn is_binary(&self) -> bool {
        self.binary
    }

    /// The unified patch, empty for a binary file.
    #[must_use]
    pub fn patch(&self) -> &str {
        &self.patch
    }
}

impl FileStatus {
    /// The status of git's `--name-status` letter; a type change counts as a modification.
    fn from_letter(letter: &[u8]) -> Option<Self> {
        match letter.first()? {
            b'A' => Some(Self::Added),
            b'M' | b'T' => Some(Self::Modified),
            b'D' => Some(Self::Deleted),
            b'R' => Some(Self::Renamed),
            _ => None,
        }
    }
}

/// The three outputs of `git diff` over one pair of commits, which list the same files in the
/// same order: `--name-status -z`, `--numstat -z` and the patch.
pub(crate) struct DiffOutputs<'a> {
    pub(crate) name_status: &'a [u8],
    pub(crate) numstat: &'a [u8],
    pub(crate) patch: &'a [u8],
}

impl DiffOutputs<'_> {
    const HEADER: &'static [u8] = b"diff --git ";

    /// The files the outputs describe, sorted by path; the reason when they do not agree.
    pub(crate) fn files(&self) -> Result<Vec<FileDiff>, &'static str> {
        let mut statuses = self.name_status.split(|byte| *byte == 0);
        let mut counts = self.numstat.split(|byte| *byte == 0);
        let mut patches = self.patches().into_iter();
        let mut files = Vec::new();
        while let Some(letter) = statuses.next().filter(|letter| !letter.is_empty()) {
            let status = FileStatus::from_letter(letter).ok_or("unknown diff status")?;
            let mut path = || {
                statuses
                    .next()
                    .and_then(|path| RepoPath::try_from(path.to_vec()).ok())
                    .ok_or("a status without a UTF-8 path")
            };
            let (previous, path) = if status == FileStatus::Renamed {
                (Some(path()?), path()?)
            } else {
                (None, path()?)
            };
            let numstat = counts.next().ok_or("fewer counts than files")?;
            if previous.is_some() {
                // A rename lists its two paths after an empty one.
                counts.next();
                counts.next();
            }
            let (additions, deletions, binary) = Self::counts(numstat)?;
            // A file that changes kind prints as a removal and an addition.
            let kind_changed = letter.first() == Some(&b'T');
            let mut text = patches.next().ok_or("fewer patches than files")?.to_vec();
            if kind_changed {
                text.extend_from_slice(patches.next().ok_or("fewer patches than files")?);
            }
            files.push(FileDiff {
                status,
                path,
                previous,
                additions,
                deletions,
                binary,
                patch: if binary {
                    String::new()
                } else {
                    String::from_utf8_lossy(&text).into_owned()
                },
            });
        }
        if patches.next().is_some() {
            return Err("more patches than files");
        }
        Ok(files)
    }

    /// The additions, deletions and binary flag of one `--numstat` record.
    fn counts(record: &[u8]) -> Result<(u64, u64, bool), &'static str> {
        let text = std::str::from_utf8(record).map_err(|_| "counts that are not text")?;
        let mut fields = text.splitn(3, '\t');
        let (Some(added), Some(deleted)) = (fields.next(), fields.next()) else {
            return Err("a malformed count");
        };
        if added == "-" && deleted == "-" {
            return Ok((0, 0, true));
        }
        let parse = |count: &str| count.parse::<u64>().map_err(|_| "a malformed count");
        Ok((parse(added)?, parse(deleted)?, false))
    }

    /// The patch split at its `diff --git` headers, which can only start a line.
    fn patches(&self) -> Vec<&[u8]> {
        let patch = self.patch;
        let mut starts = Vec::new();
        let mut line = 0;
        while line < patch.len() {
            if patch[line..].starts_with(Self::HEADER) {
                starts.push(line);
            }
            match patch[line..].iter().position(|byte| *byte == b'\n') {
                Some(end) => line += end + 1,
                None => break,
            }
        }
        starts
            .iter()
            .zip(starts.iter().skip(1).chain([&patch.len()]))
            .map(|(start, end)| &patch[*start..*end])
            .collect()
    }
}
