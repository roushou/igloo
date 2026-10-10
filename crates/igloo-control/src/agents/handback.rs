/// What a person changed in a task's sandbox while they held it, as the job reading the sandbox
/// reports it, and the prompt that names it to the tool.
///
/// The commits are those after the task's last commit carrying its `Igloo-Task` trailer (every
/// commit the tool made is given one when its turn's commits are collected), or after the task's
/// starting commit when there is none; uncommitted changes are the working tree's. Reading
/// changes nothing: the script takes no locks and commits, stashes and resets nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PersonChanges {
    commits: Vec<String>,
    stat: String,
    uncommitted: Vec<String>,
    uncommitted_stat: String,
}

/// The reading job's output is not the script's report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the output is not a report of the sandbox's changes")]
pub struct InvalidChanges;

impl PersonChanges {
    /// The variable holding the task's starting commit.
    pub const BASE: &'static str = "IGLOO_BASE";
    /// The variable holding the task's id, the value of its `Igloo-Task` trailer.
    pub const TASK: &'static str = "IGLOO_TASK";
    /// The most lines of each list the report holds.
    pub const LINES: usize = 50;

    const COMMITS: &'static str = "@@commits";
    const STAT: &'static str = "@@stat";
    const UNCOMMITTED: &'static str = "@@uncommitted";
    const UNCOMMITTED_STAT: &'static str = "@@uncommitted-stat";
    const END: &'static str = "@@end";

    /// The shell script of the reading job, run in the sandbox's working directory.
    pub const SCRIPT: &'static str = "set -e\n\
        git() { command git --no-optional-locks \"$@\"; }\n\
        mine=\"^Igloo-Task: $IGLOO_TASK\\$\"\n\
        tool=$(git log -1 --format=%H --grep=\"$mine\" \"$IGLOO_BASE..HEAD\")\n\
        since=${tool:-$IGLOO_BASE}\n\
        echo @@commits\n\
        git log --format='%h %s' --invert-grep --grep=\"$mine\" \"$since..HEAD\" | head -n 50\n\
        echo @@stat\n\
        git diff --stat \"$since\" HEAD | tail -n 50\n\
        echo @@uncommitted\n\
        git status --porcelain | head -n 50\n\
        echo @@uncommitted-stat\n\
        git diff --stat HEAD | tail -n 50\n\
        echo @@end\n";

    /// Whether the person committed or left changes uncommitted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commits.is_empty() && self.uncommitted.is_empty()
    }

    /// The prompt of the turn continuing the task: what the person changed, to be taken into
    /// account. Uncommitted changes are left in the sandbox for the tool to keep or commit.
    #[must_use]
    pub fn prompt(&self) -> String {
        let changed = if self.is_empty() {
            "Since they took it over they made no commits and left no uncommitted changes."
        } else {
            "Since they took it over they changed the sandbox."
        };
        let mut sections = vec![format!(
            "The person who took this task over has handed it back to you. {changed}"
        )];
        if !self.commits.is_empty() {
            sections.push(Self::section(
                "Their commits, newest first:",
                &self.commits,
                "Diff stat of those commits:",
                &self.stat,
            ));
        }
        if !self.uncommitted.is_empty() {
            sections.push(Self::section(
                "Their uncommitted changes, left in place in the working tree \
                 (`git status --porcelain`):",
                &self.uncommitted,
                "Diff stat of the tracked ones against HEAD:",
                &self.uncommitted_stat,
            ));
        }
        sections.push("Take their changes into account and continue with the task.".to_owned());
        sections.join("\n\n")
    }

    /// The prompt when the sandbox could not be read.
    #[must_use]
    pub fn unread_prompt() -> String {
        "The person who took this task over has handed it back to you. Igloo could not read \
         what they changed: look at `git log` and `git status` in the sandbox, take their \
         changes into account and continue with the task."
            .to_owned()
    }

    /// `title`, the `lines` as a list, and `stat` under `stat_title` when there is one.
    fn section(title: &str, lines: &[String], stat_title: &str, stat: &str) -> String {
        let list = lines
            .iter()
            .map(|line| format!("- {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        if stat.is_empty() {
            format!("{title}\n{list}")
        } else {
            format!("{title}\n{list}\n\n{stat_title}\n{stat}")
        }
    }
}

impl std::str::FromStr for PersonChanges {
    type Err = InvalidChanges;

    /// Parses the script's report: its four sections between the markers, ending with the end
    /// marker.
    fn from_str(report: &str) -> Result<Self, InvalidChanges> {
        let mut sections: [Vec<&str>; 4] = Default::default();
        let mut current: Option<usize> = None;
        let mut ended = false;
        for line in report.lines() {
            match line {
                Self::COMMITS => current = Some(0),
                Self::STAT => current = Some(1),
                Self::UNCOMMITTED => current = Some(2),
                Self::UNCOMMITTED_STAT => current = Some(3),
                Self::END => {
                    ended = true;
                    break;
                }
                _ => {
                    let Some(section) = current else {
                        return Err(InvalidChanges);
                    };
                    sections[section].push(line);
                }
            }
        }
        if !ended {
            return Err(InvalidChanges);
        }
        let [commits, stat, uncommitted, uncommitted_stat] = sections;
        let owned = |lines: Vec<&str>| lines.into_iter().map(str::to_owned).collect();
        Ok(Self {
            commits: owned(commits),
            stat: stat.join("\n"),
            uncommitted: owned(uncommitted),
            uncommitted_stat: uncommitted_stat.join("\n"),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::*;

    fn run(dir: &Path, env: &BTreeMap<&str, &str>, args: &[&str]) -> String {
        let output = std::process::Command::new(args[0])
            .args(&args[1..])
            .current_dir(dir)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .envs(env)
            .output()
            .expect("run");
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf-8")
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let mut command = vec!["git", "-c", "commit.gpgsign=false"];
        command.extend(args);
        let identity = BTreeMap::from([
            ("GIT_AUTHOR_NAME", "A"),
            ("GIT_AUTHOR_EMAIL", "a@example.invalid"),
            ("GIT_COMMITTER_NAME", "C"),
            ("GIT_COMMITTER_EMAIL", "c@example.invalid"),
        ]);
        run(dir, &identity, &command)
    }

    /// Reads `dir`'s changes as the script does for task `task` started at `base`.
    fn read(dir: &Path, base: &str, task: &str) -> PersonChanges {
        let env = BTreeMap::from([(PersonChanges::BASE, base), (PersonChanges::TASK, task)]);
        run(dir, &env, &["sh", "-c", PersonChanges::SCRIPT])
            .parse()
            .expect("a report")
    }

    #[test]
    fn the_report_lists_what_the_person_added_after_the_tools_commits_and_changes_nothing() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path();
        git(path, &["init", "--quiet", "--initial-branch=main"]);
        std::fs::write(path.join("a.txt"), "one\n").expect("write");
        git(path, &["add", "."]);
        git(path, &["commit", "--quiet", "-m", "base"]);
        let base = git(path, &["rev-parse", "HEAD"]).trim().to_owned();
        std::fs::write(path.join("a.txt"), "one\ntool\n").expect("write");
        git(
            path,
            &[
                "commit",
                "--quiet",
                "-am",
                "tool work",
                "--trailer",
                "Igloo-Task: task_1",
            ],
        );
        assert!(read(path, &base, "task_1").is_empty(), "nothing yet");

        std::fs::write(path.join("b.txt"), "person\n").expect("write");
        git(path, &["add", "b.txt"]);
        git(path, &["commit", "--quiet", "-m", "Add b"]);
        std::fs::write(path.join("a.txt"), "one\ntool\nedited\n").expect("write");
        std::fs::write(path.join("notes.txt"), "scratch\n").expect("write");
        let before = git(path, &["status", "--porcelain"]);
        let head = git(path, &["rev-parse", "HEAD"]);

        let changes = read(path, &base, "task_1");
        assert_eq!(changes.commits.len(), 1, "{changes:?}");
        assert!(changes.commits[0].ends_with(" Add b"), "{changes:?}");
        assert!(changes.stat.contains("b.txt"), "{changes:?}");
        assert!(!changes.stat.contains("a.txt"), "{changes:?}");
        assert_eq!(changes.uncommitted.len(), 2, "{changes:?}");
        assert!(changes.uncommitted_stat.contains("a.txt"), "{changes:?}");
        assert!(!changes.is_empty());

        assert_eq!(git(path, &["status", "--porcelain"]), before);
        assert_eq!(git(path, &["rev-parse", "HEAD"]), head);
        assert_eq!(
            std::fs::read_to_string(path.join("a.txt")).expect("read"),
            "one\ntool\nedited\n",
            "uncommitted work stays in place"
        );
    }

    #[test]
    fn without_a_commit_of_the_tool_every_commit_since_the_start_is_the_persons() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path();
        git(path, &["init", "--quiet", "--initial-branch=main"]);
        git(path, &["commit", "--quiet", "--allow-empty", "-m", "base"]);
        let base = git(path, &["rev-parse", "HEAD"]).trim().to_owned();
        git(path, &["commit", "--quiet", "--allow-empty", "-m", "mine"]);
        let changes = read(path, &base, "task_1");
        assert_eq!(changes.commits.len(), 1);
        assert_eq!(changes.uncommitted, Vec::<String>::new());
    }

    #[test]
    fn the_prompt_names_commits_diff_stat_and_uncommitted_changes() {
        let changes: PersonChanges = "@@commits\nabc1234 Add b\n@@stat\n b.txt | 1 +\n 1 file \
                                      changed, 1 insertion(+)\n@@uncommitted\n M a.txt\n?? \
                                      notes.txt\n@@uncommitted-stat\n a.txt | 1 +\n@@end\n"
            .parse()
            .expect("report");
        assert_eq!(
            changes.prompt(),
            "The person who took this task over has handed it back to you. Since they took it \
             over they changed the sandbox.\n\n\
             Their commits, newest first:\n- abc1234 Add b\n\n\
             Diff stat of those commits:\n b.txt | 1 +\n 1 file changed, 1 insertion(+)\n\n\
             Their uncommitted changes, left in place in the working tree \
             (`git status --porcelain`):\n-  M a.txt\n- ?? notes.txt\n\n\
             Diff stat of the tracked ones against HEAD:\n a.txt | 1 +\n\n\
             Take their changes into account and continue with the task."
        );
    }

    #[test]
    fn a_person_who_changed_nothing_says_so() {
        let changes: PersonChanges =
            "@@commits\n@@stat\n@@uncommitted\n@@uncommitted-stat\n@@end\n"
                .parse()
                .expect("report");
        assert!(changes.is_empty());
        assert_eq!(
            changes.prompt(),
            "The person who took this task over has handed it back to you. Since they took it \
             over they made no commits and left no uncommitted changes.\n\n\
             Take their changes into account and continue with the task."
        );
    }

    #[test]
    fn output_that_is_not_the_report_is_refused() {
        for output in ["", "fatal: not a git repository\n", "@@commits\nabc Add\n"] {
            assert_eq!(output.parse::<PersonChanges>(), Err(InvalidChanges));
        }
    }
}
