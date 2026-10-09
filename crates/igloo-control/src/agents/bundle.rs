use std::collections::BTreeMap;

use super::ToolName;

/// A turn's commits as they leave the sandbox: a job commits whatever the tool left
/// uncommitted, adds an `Igloo-Task` trailer naming the task to every commit since the task's
/// commit that lacks one, then writes a git bundle of those commits, base64 encoded, to its
/// standard output. Commits that already carry the trailer keep their ids. Nothing is written
/// when there are no commits.
pub struct CommitBundle;

/// Who commits in a task's sandbox: the coding tool as author, Igloo as committer. Addresses
/// are under `igloo.invalid`, a domain that never resolves, so no commit is attributed to a
/// real account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct GitIdentity {
    author: String,
    email: String,
}

/// The output is not base64.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the bundle is not valid base64")]
pub struct InvalidBundle;

impl GitIdentity {
    /// The committer of every commit made or rewritten in a task's sandbox.
    pub(super) const COMMITTER: (&'static str, &'static str) = ("Igloo", "igloo@igloo.invalid");

    /// `tool` as author: `<tool> (Igloo)` at `<tool>@igloo.invalid`.
    #[must_use]
    pub(super) fn of(tool: &ToolName) -> Self {
        Self {
            author: format!("{tool} (Igloo)"),
            email: format!("{tool}@igloo.invalid"),
        }
    }

    /// The identity as git's environment variables.
    #[must_use]
    pub(super) fn env(&self) -> BTreeMap<String, String> {
        let (committer, committer_email) = Self::COMMITTER;
        BTreeMap::from([
            ("GIT_AUTHOR_NAME".to_owned(), self.author.clone()),
            ("GIT_AUTHOR_EMAIL".to_owned(), self.email.clone()),
            ("GIT_COMMITTER_NAME".to_owned(), committer.to_owned()),
            ("GIT_COMMITTER_EMAIL".to_owned(), committer_email.to_owned()),
        ])
    }
}

impl CommitBundle {
    /// The variable holding the task's commit.
    pub const BASE: &'static str = "IGLOO_BASE";
    /// The variable holding the message of the commit of uncommitted work.
    pub const MESSAGE: &'static str = "IGLOO_MESSAGE";
    /// The variable holding the task's id, written as the `Igloo-Task` trailer.
    pub const TASK: &'static str = "IGLOO_TASK";

    /// The shell script of the job, run in the sandbox's working directory.
    pub const SCRIPT: &'static str = "set -e\n\
        if [ -n \"$(git status --porcelain)\" ]; then\n\
        \x20 git add --all && git commit --quiet --message \"$IGLOO_MESSAGE\"\n\
        fi\n\
        if [ \"$(git rev-parse HEAD)\" != \"$IGLOO_BASE\" ]; then\n\
        \x20 git rebase --quiet --exec 'git log -1 --format=%B | grep -q \"^Igloo-Task: \" \
        || git commit --quiet --amend --allow-empty --no-edit --trailer \"Igloo-Task: $IGLOO_TASK\"' \
        \"$IGLOO_BASE\"\n\
        \x20 git bundle create --quiet .git/igloo.bundle \"$IGLOO_BASE..HEAD\"\n\
        \x20 base64 < .git/igloo.bundle\n\
        \x20 rm .git/igloo.bundle\n\
        fi\n";

    /// The bundle in the job's standard output, or `None` when the turn made no commits.
    pub fn decode(output: &[u8]) -> Result<Option<Vec<u8>>, InvalidBundle> {
        let sextets: Vec<u8> = output
            .iter()
            .filter(|byte| !byte.is_ascii_whitespace() && **byte != b'=')
            .map(|byte| Self::sextet(*byte))
            .collect::<Result<_, _>>()?;
        if sextets.is_empty() {
            return Ok(None);
        }
        if sextets.len() % 4 == 1 {
            return Err(InvalidBundle);
        }
        let mut bytes = Vec::with_capacity(sextets.len() * 3 / 4);
        for chunk in sextets.chunks(4) {
            let block = chunk
                .iter()
                .enumerate()
                .fold(0u32, |block, (index, sextet)| {
                    block | (u32::from(*sextet) << (18 - 6 * index))
                });
            let [_, a, b, c] = block.to_be_bytes();
            bytes.extend([a, b, c].into_iter().take(chunk.len() - 1));
        }
        Ok(Some(bytes))
    }

    fn sextet(byte: u8) -> Result<u8, InvalidBundle> {
        match byte {
            b'A'..=b'Z' => Ok(byte - b'A'),
            b'a'..=b'z' => Ok(byte - b'a' + 26),
            b'0'..=b'9' => Ok(byte - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(InvalidBundle),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_base64_decodes_and_no_output_means_no_commits() {
        assert_eq!(CommitBundle::decode(b""), Ok(None));
        assert_eq!(CommitBundle::decode(b" \n"), Ok(None));
        assert_eq!(
            CommitBundle::decode(b"aGVsbG8g\nd29ybGQ=\n"),
            Ok(Some(b"hello world".to_vec()))
        );
        assert_eq!(CommitBundle::decode(b"YQ=="), Ok(Some(b"a".to_vec())));
        assert_eq!(CommitBundle::decode(b"YWI="), Ok(Some(b"ab".to_vec())));
        assert_eq!(CommitBundle::decode(b"not base64!"), Err(InvalidBundle));
        assert_eq!(CommitBundle::decode(b"YWJjZ"), Err(InvalidBundle));
    }

    /// Runs `args` in `dir` with `env`; returns standard output.
    fn run(dir: &std::path::Path, env: &BTreeMap<String, String>, args: &[&str]) -> String {
        let output = std::process::Command::new(args[0])
            .args(&args[1..])
            .current_dir(dir)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .envs(env)
            .output()
            .expect("spawn");
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    #[test]
    fn collected_commits_carry_the_tool_igloo_and_the_task_once() {
        let dir = tempfile::tempdir().expect("dir");
        let tool: ToolName = "claude-code".parse().expect("tool");
        let identity = GitIdentity::of(&tool).env();
        let git = |args: &[&str]| run(dir.path(), &identity, &[&["git"], args].concat());
        git(&["init", "--quiet", "--initial-branch=main"]);
        git(&["commit", "--quiet", "--allow-empty", "--message", "base"]);
        let base = git(&["rev-parse", "HEAD"]).trim().to_owned();
        git(&[
            "commit",
            "--quiet",
            "--allow-empty",
            "--message",
            "earlier turn\n\nIgloo-Task: task_1",
        ]);
        let earlier = git(&["rev-parse", "HEAD"]).trim().to_owned();
        git(&[
            "commit",
            "--quiet",
            "--allow-empty",
            "--message",
            "this turn",
        ]);
        std::fs::write(dir.path().join("left.txt"), "uncommitted").expect("write");

        let env = identity
            .clone()
            .into_iter()
            .chain([
                (CommitBundle::BASE.to_owned(), base.clone()),
                (CommitBundle::MESSAGE.to_owned(), "the goal".to_owned()),
                (CommitBundle::TASK.to_owned(), "task_1".to_owned()),
            ])
            .collect();
        let stdout = run(dir.path(), &env, &["sh", "-c", CommitBundle::SCRIPT]);

        let bundle = CommitBundle::decode(stdout.as_bytes())
            .expect("base64")
            .expect("commits");
        assert!(bundle.starts_with(b"# v"), "a git bundle");
        let log = git(&[
            "log",
            "--format=%H|%an <%ae>|%cn <%ce>|%s|%(trailers:key=Igloo-Task,valueonly,separator=%x2C)",
            &format!("{base}..HEAD"),
        ]);
        let commits: Vec<Vec<&str>> = log.lines().map(|line| line.split('|').collect()).collect();
        assert_eq!(
            commits.iter().map(|commit| commit[3]).collect::<Vec<_>>(),
            ["the goal", "this turn", "earlier turn"]
        );
        for commit in &commits {
            assert_eq!(commit[1], "claude-code (Igloo) <claude-code@igloo.invalid>");
            assert_eq!(commit[2], "Igloo <igloo@igloo.invalid>");
            assert_eq!(commit[4], "task_1", "one trailer on {}", commit[3]);
        }
        assert_eq!(
            commits[2][0], earlier,
            "a commit with the trailer keeps its id"
        );
    }
}
