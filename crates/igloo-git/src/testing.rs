//! A git origin on disk for tests: a bare repository and a work tree that pushes to it.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    reason = "a test fixture reports failures by panicking"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use igloo_core::repo::{CommitId, RepoLocation};

/// A bare `origin.git` and a `work` tree whose commits are pushed to it, both on `main`.
///
/// Invariant: git runs without global or system configuration and commits as `igloo`, so the
/// fixture behaves the same on every machine.
pub struct Fixture {
    origin: PathBuf,
    work: PathBuf,
}

impl Fixture {
    /// An empty origin and work tree under `dir`.
    pub fn new(dir: &Path) -> Self {
        let fixture = Self {
            origin: dir.join("origin.git"),
            work: dir.join("work"),
        };
        Self::run(
            dir,
            &[
                "init",
                "--quiet",
                "--bare",
                "--initial-branch=main",
                "origin.git",
            ],
        );
        Self::run(dir, &["init", "--quiet", "--initial-branch=main", "work"]);
        fixture
    }

    /// The bare origin.
    pub fn origin(&self) -> &Path {
        &self.origin
    }

    /// The work tree.
    pub fn work(&self) -> &Path {
        &self.work
    }

    /// The origin as a repository location.
    pub fn location(&self) -> RepoLocation {
        RepoLocation::Local {
            path: self.origin.display().to_string(),
        }
    }

    /// Switches the work tree to `branch`, creating or resetting it at the current commit.
    pub fn switch(&self, branch: &str) {
        self.git(&["checkout", "--quiet", "-B", branch]);
    }

    /// Writes (`Some`) or deletes (`None`) `files` in the work tree, commits everything with
    /// `message` and force-pushes the current branch to the origin; returns the commit.
    pub fn commit(&self, message: &str, files: &[(&str, Option<&str>)]) -> CommitId {
        for (path, content) in files {
            let target = self.work.join(path);
            match content {
                Some(content) => {
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent).expect("create the file's directory");
                    }
                    std::fs::write(&target, content).expect("write the file");
                }
                None => std::fs::remove_file(&target).expect("delete the file"),
            }
        }
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "--allow-empty", "-m", message]);
        let origin = self.origin.display().to_string();
        self.git(&["push", "--quiet", "--force", &origin, "HEAD"]);
        self.git(&["rev-parse", "HEAD"])
            .parse()
            .expect("a commit id")
    }

    /// The commit `branch` is at in the origin.
    pub fn head(&self, branch: &str) -> CommitId {
        Self::run(
            &self.origin,
            &["rev-parse", &format!("refs/heads/{branch}")],
        )
        .parse()
        .expect("a commit id")
    }

    /// Runs git with `args` in the work tree and returns its trimmed output.
    pub fn git(&self, args: &[&str]) -> String {
        Self::run(&self.work, args)
    }

    fn run(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "igloo")
            .env("GIT_AUTHOR_EMAIL", "igloo@example.com")
            .env("GIT_COMMITTER_NAME", "igloo")
            .env("GIT_COMMITTER_EMAIL", "igloo@example.com")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}
