//! Setting up a new workspace's sandbox: its checkout's `origin` points at Igloo's git endpoint
//! and the repository's dotfiles are installed, once, in a job that runs when the sandbox first
//! runs.

use std::sync::Arc;

use async_trait::async_trait;
use igloo_core::job::{Job, JobEvent, JobSpec, JobTimeout};
use igloo_core::process::{Argv, EnvVars};
use igloo_core::repo::{Repo, RepoId};
use igloo_core::sandbox::SandboxId;
use igloo_core::workspace::{Workspace, WorkspaceEvent};
use igloo_core::{Actor, Entity as _, Resource as _, SystemComponent, ValidationErrors};
use jiff::SignedDuration;
use reqwest::Url;
use std::collections::BTreeSet;

use super::commands::WorkspaceQueries;
use crate::app::{AppError, CommandBus, Reactor, RequestContext};
use crate::platform::SubmitJob;
use crate::ports::{EntityStore, EventEnvelope};

/// The job that sets up a workspace's sandbox.
///
/// Invariants: the job runs at most once to completion per sandbox filesystem (a marker file in
/// the sandbox, sealed with the workspace, makes repeats no-ops); it carries no credentials;
/// pointing `origin` at Igloo and installing the dotfiles fail independently, and a failure of
/// either fails the job and nothing else.
#[derive(Clone)]
pub(super) struct WorkspaceSetup {
    git_base: Url,
}

impl WorkspaceSetup {
    /// The `$0` of the script, which identifies setup jobs.
    const NAME: &'static str = "igloo-workspace-setup";
    const TIMEOUT: SignedDuration = SignedDuration::from_mins(15);
    const SCRIPT: &'static str = r#"
marker=.git/igloo-workspace-setup
log=.git/igloo-setup.log
[ -e "$marker" ] && exit 0
(
    status=0
    if [ -n "$IGLOO_GIT_URL" ]; then
        git remote remove origin >/dev/null 2>&1
        if ! { git remote add origin "$IGLOO_GIT_URL" \
            && git checkout -q -B "$IGLOO_WORKSPACE_BRANCH" \
            && git config "branch.$IGLOO_WORKSPACE_BRANCH.remote" origin \
            && git config "branch.$IGLOO_WORKSPACE_BRANCH.merge" "refs/heads/$IGLOO_WORKSPACE_BRANCH"; }; then
            echo "igloo: pointing origin at Igloo failed" >&2
            status=1
        fi
    fi
    if [ -n "$IGLOO_DOTFILES_REPOSITORY" ]; then
        dotfiles="${HOME:-/root}/.dotfiles"
        if ! { git clone --quiet -- "$IGLOO_DOTFILES_REPOSITORY" "$dotfiles" \
            && cd "$dotfiles" && sh -c "$IGLOO_DOTFILES_INSTALL"; }; then
            echo "igloo: installing the dotfiles failed" >&2
            status=1
        fi
    fi
    exit $status
) > "$log" 2>&1
status=$?
cat "$log"
touch "$marker"
exit $status
"#;

    /// Setup whose `origin` is `repo`'s path under `git_base`, Igloo's public URL.
    pub(super) fn new(git_base: Url) -> Self {
        Self { git_base }
    }

    /// The job setting up `sandbox`, the sandbox of `workspace`, for `repo`.
    pub(super) fn job(
        &self,
        workspace: &Workspace,
        repo: &Repo,
        sandbox: SandboxId,
    ) -> Result<JobSpec, ValidationErrors> {
        let mut env = vec![
            ("IGLOO_GIT_URL".to_owned(), self.git_url(repo.id())),
            (
                "IGLOO_WORKSPACE_BRANCH".to_owned(),
                workspace.spec().branch.to_string(),
            ),
        ];
        if let Some(dotfiles) = repo.dotfiles() {
            env.push((
                "IGLOO_DOTFILES_REPOSITORY".to_owned(),
                dotfiles.repository().to_owned(),
            ));
            env.push((
                "IGLOO_DOTFILES_INSTALL".to_owned(),
                dotfiles.install().to_owned(),
            ));
        }
        let argv = Argv::try_from(vec![
            "sh".to_owned(),
            "-c".to_owned(),
            Self::SCRIPT.to_owned(),
            Self::NAME.to_owned(),
        ])
        .map_err(|error| ValidationErrors::single("argv", error.to_string()))?;
        Ok(JobSpec::Execute {
            sandbox,
            argv,
            env: EnvVars::from_pairs(env)?,
            secrets: BTreeSet::new(),
            timeout: JobTimeout::try_from(Self::TIMEOUT)
                .map_err(|error| ValidationErrors::single("timeout", error.to_string()))?,
        })
    }

    /// Whether `spec` is a setup job.
    pub(super) fn is_setup(spec: &JobSpec) -> bool {
        let JobSpec::Execute { argv, .. } = spec;
        argv.args().last().is_some_and(|name| name == Self::NAME)
    }

    /// Where Igloo serves `repo`'s git: `<base>/git/<repo>.git`.
    fn git_url(&self, repo: RepoId) -> String {
        let mut url = self.git_base.clone();
        if let Ok(mut segments) = url.path_segments_mut() {
            segments
                .pop_if_empty()
                .extend(["git", &format!("{repo}.git")]);
        }
        url.to_string()
    }
}

fn context(event: &EventEnvelope) -> RequestContext {
    RequestContext::caused_by(
        event,
        Actor::System {
            component: SystemComponent::Reactor,
        },
    )
}

/// Submits the setup job when a workspace's sandbox runs for the first time: a workspace that
/// resumes from a sealed snapshot was set up already.
pub(super) struct SetUpWorkspaces {
    pub(super) setup: WorkspaceSetup,
    pub(super) workspaces: WorkspaceQueries,
    pub(super) repos: Arc<dyn EntityStore<Repo>>,
}

#[async_trait]
impl Reactor for SetUpWorkspaces {
    fn name(&self) -> &'static str {
        "workspace.set_up"
    }

    async fn react(&self, event: &EventEnvelope, bus: &CommandBus) -> Result<(), AppError> {
        let Some(id) = event.subject_as::<Workspace>() else {
            return Ok(());
        };
        let WorkspaceEvent::SandboxRunning { sandbox, .. } = event.decode::<WorkspaceEvent>()?
        else {
            return Ok(());
        };
        let Some(workspace) = self.workspaces.get(id).await? else {
            return Ok(());
        };
        let current = workspace.status().sandbox() == Some(sandbox);
        if !current || workspace.status().snapshot().is_some() {
            return Ok(());
        }
        let Some(repo) = self.repos.load(workspace.spec().repo).await? else {
            return Ok(());
        };
        let spec = self.setup.job(&workspace, repo.entity(), sandbox)?;
        bus.dispatch(SubmitJob { spec }, context(event)).await?;
        Ok(())
    }
}

/// Reports a setup job that did not succeed: the owner's workspace is still usable, and the
/// job's output is in the job's logs and in `.git/igloo-setup.log` of the checkout.
pub(super) struct ReportSetups {
    pub(super) jobs: Arc<dyn EntityStore<Job>>,
}

#[async_trait]
impl Reactor for ReportSetups {
    fn name(&self) -> &'static str {
        "workspace.report_setup"
    }

    async fn react(&self, event: &EventEnvelope, _bus: &CommandBus) -> Result<(), AppError> {
        let Some(id) = event.subject_as::<Job>() else {
            return Ok(());
        };
        let ending = match event.decode::<JobEvent>()? {
            JobEvent::Finished { exit_code: 0 } => return Ok(()),
            JobEvent::Finished { exit_code } => format!("exited with {exit_code}"),
            JobEvent::Failed { reason } => format!("did not complete: {reason}"),
            _ => return Ok(()),
        };
        let Some(job) = self.jobs.load(id).await? else {
            return Ok(());
        };
        if WorkspaceSetup::is_setup(job.entity().spec()) {
            tracing::warn!(
                job = %id,
                sandbox = %job.entity().spec().sandbox(),
                "workspace setup {ending}; see the job's logs"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Output;

    use igloo_git::testing::Fixture;

    use super::*;

    /// Runs the setup script as a sandbox would, over `checkout` with `home` as the user's home.
    async fn run(checkout: &Path, home: &Path, extra: &[(&str, &str)]) -> Output {
        let mut command = tokio::process::Command::new("sh");
        command.current_dir(checkout);
        command
            .args(["-c", WorkspaceSetup::SCRIPT, WorkspaceSetup::NAME])
            .env("HOME", home)
            .env("IGLOO_WORKSPACE_BRANCH", "feature")
            .env("IGLOO_GIT_URL", "http://igloo.test/git/repo_1.git")
            .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1");
        for (name, value) in extra {
            command.env(name, value);
        }
        command.output().await.expect("sh")
    }

    fn fixture(parent: &Path, name: &str) -> Fixture {
        let dir = parent.join(name);
        std::fs::create_dir(&dir).expect("dir");
        Fixture::new(&dir)
    }

    fn remote(checkout: &Path) -> String {
        let output = std::process::Command::new("git")
            .current_dir(checkout)
            .args(["config", "remote.origin.url"])
            .output()
            .expect("git");
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    #[tokio::test]
    async fn the_script_points_origin_at_igloo_installs_the_dotfiles_and_runs_once() {
        let dir = tempfile::tempdir().expect("dir");
        let work = fixture(dir.path(), "a");
        work.commit("first", &[("a", Some("a"))]);
        let dotfiles = fixture(dir.path(), "b");
        dotfiles.commit("dots", &[(".vimrc", Some("set number"))]);
        let url = format!("file://{}", dotfiles.work().display());
        let home = dir.path().join("home");
        std::fs::create_dir(&home).expect("home");

        let env = [
            ("IGLOO_DOTFILES_REPOSITORY", url.as_str()),
            ("IGLOO_DOTFILES_INSTALL", "cp .vimrc \"$HOME/.vimrc\""),
        ];
        let output = run(work.work(), &home, &env).await;
        assert!(output.status.success(), "{output:?}");
        assert_eq!(remote(work.work()), "http://igloo.test/git/repo_1.git");
        let branch = work.git(&["rev-parse", "--abbrev-ref", "HEAD"]);
        assert_eq!(branch.trim(), "feature");
        assert_eq!(
            work.git(&["config", "branch.feature.merge"]).trim(),
            "refs/heads/feature"
        );
        assert_eq!(
            std::fs::read_to_string(home.join(".vimrc")).expect("installed"),
            "set number"
        );

        std::fs::remove_file(home.join(".vimrc")).expect("remove");
        let again = run(work.work(), &home, &env).await;
        assert!(again.status.success());
        assert!(!home.join(".vimrc").exists(), "a second run does nothing");
    }

    #[tokio::test]
    async fn a_failing_install_fails_the_job_but_leaves_origin_set_and_is_not_retried() {
        let dir = tempfile::tempdir().expect("dir");
        let work = fixture(dir.path(), "a");
        work.commit("first", &[("a", Some("a"))]);
        let dotfiles = fixture(dir.path(), "b");
        dotfiles.commit("dots", &[(".vimrc", Some("x"))]);
        let url = format!("file://{}", dotfiles.work().display());
        let home = dir.path().join("home");
        std::fs::create_dir(&home).expect("home");

        let env = [
            ("IGLOO_DOTFILES_REPOSITORY", url.as_str()),
            ("IGLOO_DOTFILES_INSTALL", "echo broken >&2; exit 3"),
        ];
        let output = run(work.work(), &home, &env).await;
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("broken"));
        assert_eq!(remote(work.work()), "http://igloo.test/git/repo_1.git");
        let log = std::fs::read_to_string(work.work().join(".git/igloo-setup.log")).expect("log");
        assert!(log.contains("installing the dotfiles failed"), "{log}");

        let again = run(work.work(), &home, &env).await;
        assert!(again.status.success(), "the marker makes a repeat a no-op");
    }

    #[tokio::test]
    async fn the_script_does_nothing_it_was_not_asked_to() {
        let dir = tempfile::tempdir().expect("dir");
        let work = fixture(dir.path(), "a");
        work.commit("first", &[("a", Some("a"))]);
        let home = dir.path().join("home");
        std::fs::create_dir(&home).expect("home");
        let output = run(work.work(), &home, &[("IGLOO_GIT_URL", "")]).await;
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            remote(work.work()),
            "",
            "origin is left alone without a URL"
        );
        assert!(!home.join(".dotfiles").exists());
    }
}
