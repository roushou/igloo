//! Igloo hosts git: stock git clones, fetches and pushes against the server with the API token,
//! a push opens a change, the default branch moves only by merge, and the forge is a mirror.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "a test reports failures by panicking"
)]

use std::path::Path;
use std::time::Duration;

use igloo::change::ChangePhase;
use igloo::repo::RegisterRepoRequest;

use self::common::{DEV_TOKEN, EndToEnd, Origin};

mod common;

/// Runs the stock `git` in `dir`; returns whether it succeeded and what it printed.
fn git(dir: &Path, args: &[&str]) -> (bool, String) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@igloo.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@igloo.invalid")
        .output()
        .expect("git");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), said)
}

fn git_ok(dir: &Path, args: &[&str]) -> String {
    let (ok, said) = git(dir, args);
    assert!(ok, "git {args:?}: {said}");
    said.trim().to_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn git_clones_fetches_and_pushes_through_igloo() {
    let end_to_end = EndToEnd::start().await;
    let client = &end_to_end.client;
    let origin = Origin::new();
    origin.commit(&[("README.md", "igloo")]);
    let repo = client
        .register_repo(&RegisterRepoRequest::new(origin.path.clone()))
        .await
        .expect("register");
    let address = end_to_end.server.rest_address();
    let url = format!("http://{address}{}", repo.git_path);
    let with_token = format!("http://anyone:{DEV_TOKEN}@{address}{}", repo.git_path);
    let scratch = tempfile::tempdir().expect("dir");

    // Igloo's copy appears once the repository is registered; until it holds the default branch,
    // git is refused rather than given an empty repository.
    let mut cloned = false;
    for _ in 0..100 {
        if git(scratch.path(), &["clone", "--quiet", &with_token, "first"]).0 {
            cloned = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(cloned, "the repository is hosted after registration");
    // Only a populated copy is served, so the clone holds the origin's commit.
    assert_eq!(
        git_ok(&scratch.path().join("first"), &["rev-parse", "HEAD"]),
        origin.head("main")
    );
    assert_eq!(
        std::fs::read_to_string(scratch.path().join("first").join("README.md")).expect("file"),
        "igloo"
    );

    // Without the token, or with another, git is refused.
    for refused in [
        url.clone(),
        format!("http://anyone:wrong@{address}{}", repo.git_path),
    ] {
        let (ok, said) = git(scratch.path(), &["clone", "--quiet", &refused, "denied"]);
        assert!(!ok, "{said}");
    }

    // A credential helper supplies the token, as `igloo git-credential` does.
    let helper =
        format!("credential.helper=!f() {{ echo username=igloo; echo password={DEV_TOKEN}; }}; f");
    git_ok(
        scratch.path(),
        &["clone", "--quiet", "--config", &helper, &url, "helped"],
    );

    // A pushed branch opens a change titled with its head's subject; a later push revises it.
    let work = scratch.path().join("first");
    git_ok(&work, &["switch", "--quiet", "-c", "feature"]);
    std::fs::write(work.join("a"), "a").expect("write");
    git_ok(&work, &["add", "."]);
    git_ok(&work, &["commit", "--quiet", "-m", "Add a"]);
    git_ok(&work, &["push", "--quiet", "origin", "feature"]);
    let changes = client.list_changes(&repo.id).await.expect("changes");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].title, "Add a");
    assert_eq!(changes[0].source_branch, "feature");
    assert_eq!(changes[0].phase, ChangePhase::Open);
    std::fs::write(work.join("b"), "b").expect("write");
    git_ok(&work, &["add", "."]);
    git_ok(&work, &["commit", "--quiet", "-m", "Add b"]);
    git_ok(&work, &["push", "--quiet", "origin", "feature"]);
    let changes = client.list_changes(&repo.id).await.expect("changes");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].revisions.len(), 2);
    let head = git_ok(&work, &["rev-parse", "HEAD"]);
    assert_eq!(changes[0].revisions[1].head, head);

    // The change's branch is mirrored to the forge while the change is open.
    origin.reaches("feature", &head).await;

    // A fetch sees what is in Igloo.
    git_ok(
        scratch.path().join("helped").as_path(),
        &["fetch", "--quiet"],
    );
    assert_eq!(
        git_ok(
            &scratch.path().join("helped"),
            &["rev-parse", "origin/feature"]
        ),
        head
    );

    // The default branch moves only by merge.
    git_ok(&work, &["switch", "--quiet", "main"]);
    std::fs::write(work.join("c"), "c").expect("write");
    git_ok(&work, &["add", "."]);
    git_ok(&work, &["commit", "--quiet", "-m", "Straight to main"]);
    let (ok, said) = git(&work, &["push", "origin", "main"]);
    assert!(!ok, "{said}");
    assert!(said.contains("igloo change merge"), "{said}");

    end_to_end.stop().await;
}
