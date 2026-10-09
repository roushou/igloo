//! End to end against a real Postgres and the embedded worker: a local directory runs as a
//! job, and the sandbox it ran in is sealed and forked.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "the helpers of a test report failures by panicking"
)]

use std::time::Duration;

use igloo::change::{ChangePhase, OpenChangeRequest};
use igloo::ci::{CheckStatus, RunPhase};
use igloo::job::ExecRequest;
use igloo::repo::{RegisterRepoRequest, RepoSnapshotRequest};
use igloo::sandbox::{CreateSandboxRequest, DesiredState};
use igloo::{Client, Run};

use self::common::{EndToEnd, Origin, Output, ended_run, refusal};

mod common;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_local_directory_runs_on_a_worker_and_its_sandbox_seals_and_forks() {
    let end_to_end = EndToEnd::start().await;
    let client = &end_to_end.client;

    let project = tempfile::tempdir().expect("project");
    std::fs::write(project.path().join(".gitignore"), "secret.txt\n").expect("write");
    std::fs::write(project.path().join("hello.txt"), "hello from igloo\n").expect("write");
    std::fs::write(project.path().join("secret.txt"), "never uploaded\n").expect("write");

    let script = "cat hello.txt; echo built > artifact; rm hello.txt; \
                  test -e secret.txt || echo secret ignored >&2; exit 3";
    let run = Run::new(vec!["sh".to_owned(), "-c".to_owned(), script.to_owned()]);
    let mut running =
        tokio::time::timeout(Duration::from_secs(30), client.run(project.path(), run))
            .await
            .expect("started within 30 s")
            .expect("run");
    let output = Output::of(running.logs(), Duration::from_secs(60)).await;
    assert_eq!(output.stdout, "hello from igloo\n");
    assert_eq!(output.stderr, "secret ignored\n");
    assert_eq!(output.exit_code(), Some(3));

    let snapshot = tokio::time::timeout(Duration::from_secs(30), client.seal(&running.sandbox.id))
        .await
        .expect("sealed within 30 s")
        .expect("seal");
    let fork = client
        .create_sandbox(&CreateSandboxRequest::new(snapshot))
        .await
        .expect("fork");
    let check = ExecRequest::new(vec![
        "sh".to_owned(),
        "-c".to_owned(),
        "cat artifact; test ! -e hello.txt".to_owned(),
    ]);
    let job = client
        .exec(&fork.id, &check)
        .await
        .expect("exec in the fork");
    let output = Output::of(
        &mut client.logs(&job.id).await.expect("logs"),
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(output.stdout, "built\n", "the fork sees the sealed files");
    assert_eq!(output.exit_code(), Some(0), "and the sealed deletion");

    let stopped = client
        .stop_sandbox(&running.sandbox.id)
        .await
        .expect("stop");
    assert_eq!(stopped.id, running.sandbox.id);

    let kinds: Vec<String> = sqlx::query_scalar("SELECT type FROM events ORDER BY sequence")
        .fetch_all(end_to_end.server.database().pool())
        .await
        .expect("events");
    for kind in [
        "igloo.worker.registered",
        "igloo.sandbox.created",
        "igloo.sandbox.scheduled",
        "igloo.job.submitted",
        "igloo.job.leased",
        "igloo.job.finished",
        "igloo.sandbox.stop_requested",
        "igloo.seal.requested",
        "igloo.seal.sealed",
    ] {
        assert!(
            kinds.iter().any(|recorded| recorded == kind),
            "missing {kind} in {kinds:?}"
        );
    }
    end_to_end
        .server
        .shutdown(Duration::from_secs(10))
        .await
        .expect("shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_repository_snapshot_holds_its_checkout_and_commit() {
    let end_to_end = EndToEnd::start().await;
    let client = &end_to_end.client;
    let origin = Origin::new();
    let head = origin.commit(&[("README.md", "igloo\n")]);
    let repo = client
        .register_repo(&RegisterRepoRequest::new(origin.path.clone()))
        .await
        .expect("register");
    let snapshot = client
        .snapshot_repo(&repo.id, &RepoSnapshotRequest::branch("main"))
        .await
        .expect("snapshot");
    assert_eq!(snapshot.commit, head);
    let sandbox = client
        .create_sandbox(&CreateSandboxRequest::new(snapshot.snapshot))
        .await
        .expect("sandbox");
    let check = ExecRequest::new(vec![
        "sh".to_owned(),
        "-c".to_owned(),
        "git log -1 --format=%H && cat README.md".to_owned(),
    ]);
    let job = client.exec(&sandbox.id, &check).await.expect("exec");
    let output = Output::of(
        &mut client.logs(&job.id).await.expect("logs"),
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(output.exit_code(), Some(0), "{}", output.stderr);
    assert_eq!(output.stdout, format!("{head}\nigloo\n"));
    end_to_end.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_revision_runs_its_checks_over_a_warm_snapshot_built_once() {
    let end_to_end = EndToEnd::start().await;
    let client = &end_to_end.client;
    let origin = Origin::new();
    let pipeline = "[warm]\ncommand = \"echo warmed > warm.txt\"\nlockfiles = [\"deps.lock\"]\n\n\
                    [[checks]]\nname = \"warm\"\nrun = \"grep -q warmed warm.txt\"\n\n\
                    [[checks]]\nname = \"fails\"\nrun = \"exit 3\"\n";
    origin.commit(&[(".igloo/pipeline.toml", pipeline), ("deps.lock", "v1")]);
    origin.branch("feature");
    origin.commit(&[("src.txt", "one")]);
    let repo = client
        .register_repo(&RegisterRepoRequest::new(origin.path.clone()))
        .await
        .expect("register");
    let change = client
        .open_change(
            &repo.id,
            &OpenChangeRequest::new("feature", "Change things"),
        )
        .await
        .expect("open");
    let first = ended_run(client, &change.id, 1).await;
    assert_eq!(first.phase, RunPhase::Failed, "{first:?}");
    assert_eq!(
        first.checks[0].status,
        CheckStatus::Passed,
        "the warm snapshot is used"
    );
    assert_eq!(first.checks[1].status, CheckStatus::Failed);
    assert_eq!(first.checks[1].exit_code, Some(3));

    origin.commit(&[("src.txt", "two")]);
    client.revise_change(&change.id).await.expect("revise");
    let second = ended_run(client, &change.id, 2).await;
    assert_eq!(second.checks[0].status, CheckStatus::Passed, "{second:?}");
    let warm_builds: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE type = 'igloo.build.requested'")
            .fetch_one(end_to_end.server.database().pool())
            .await
            .expect("count");
    assert_eq!(
        warm_builds, 1,
        "the second revision reuses the warm snapshot"
    );
    sandboxes_stop(client).await;
    end_to_end.stop().await;
}

/// Waits up to ten seconds for every sandbox to be asked to stop.
async fn sandboxes_stop(client: &Client) {
    let all_stopped = async || {
        client
            .list_sandboxes(&[], None)
            .await
            .expect("sandboxes")
            .items
            .iter()
            .all(|sandbox| sandbox.desired == DesiredState::Stopped)
    };
    for _ in 0..100 {
        if all_stopped().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("ended runs stop their sandboxes");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_change_merges_once_its_checks_pass_and_protected_paths_are_approved() {
    let end_to_end = EndToEnd::start().await;
    let client = &end_to_end.client;
    let origin = Origin::new();
    origin.commit(&[
        (
            ".igloo/pipeline.toml",
            "[[checks]]\nname = \"ok\"\nrun = \"test -f ok\"\n",
        ),
        (".igloo/agents.toml", "protected = [\"secure/**\"]\n"),
    ]);
    origin.branch("feature");
    origin.commit(&[("work.txt", "one")]);
    let repo = client
        .register_repo(&RegisterRepoRequest::new(origin.path.clone()))
        .await
        .expect("register");
    let change = client
        .open_change(
            &repo.id,
            &OpenChangeRequest::new("feature", "Secure things"),
        )
        .await
        .expect("open");
    ended_run(client, &change.id, 1).await;
    assert_eq!(
        refusal(client, &change.id).await,
        "change.checks_not_passed"
    );

    origin.commit(&[("ok", ""), ("secure/key", "k")]);
    client.revise_change(&change.id).await.expect("revise");
    assert_eq!(
        ended_run(client, &change.id, 2).await.phase,
        RunPhase::Passed
    );
    assert_eq!(
        refusal(client, &change.id).await,
        "change.approval_required"
    );
    client.approve_change(&change.id).await.expect("approve");

    origin.git(&["checkout", "--quiet", "main"]);
    origin.commit(&[("elsewhere.txt", "moved")]);
    assert_eq!(refusal(client, &change.id).await, "change.target_moved");

    origin.git(&["checkout", "--quiet", "feature"]);
    origin.git(&[
        "-c",
        "user.name=igloo",
        "-c",
        "user.email=igloo@example.com",
        "rebase",
        "--quiet",
        "main",
    ]);
    let rebased = origin.git(&["rev-parse", "HEAD"]);
    origin.git(&["push", "--quiet", "--force", &origin.path, "feature"]);
    client.revise_change(&change.id).await.expect("revise");
    assert_eq!(
        ended_run(client, &change.id, 3).await.phase,
        RunPhase::Passed
    );
    assert_eq!(
        refusal(client, &change.id).await,
        "change.approval_required",
        "an approval holds for its revision only"
    );
    client.approve_change(&change.id).await.expect("approve");
    let merged = client.merge_change(&change.id).await.expect("merge");
    assert_eq!(merged.phase, ChangePhase::Merged);
    let main = origin.head("main");
    assert_eq!(
        merged.merged_commit.as_deref(),
        Some(main.as_str()),
        "Igloo pushed the target branch"
    );
    let tree = |commit: &str| {
        origin.git(&[
            "-C",
            &origin.path,
            "rev-parse",
            &format!("{commit}^{{tree}}"),
        ])
    };
    assert_eq!(
        tree(&main),
        tree(&rebased),
        "one squashed commit of the revision"
    );
    let again = client.merge_change(&change.id).await.expect("merge again");
    assert_eq!(again.phase, ChangePhase::Merged);
    end_to_end.stop().await;
}
