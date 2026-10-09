//! Tasks against a git repository on disk: a scripted tool works in a sandbox forked from the
//! repository's agent snapshot, built once for every task using the tool.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "a test reports failures by panicking"
)]

use std::time::Duration;

use igloo::Client;
use igloo::change::CommentRequest;
use igloo::ci::RunPhase;
use igloo::repo::RegisterRepoRequest;
use igloo::sandbox::DesiredState;
use igloo::task::{CreateTaskRequest, TaskPhase, TaskResource, TranscriptItem, TurnStatus};

use self::common::{EndToEnd, Origin, ended_run};

mod common;

const SECRET: &str = "s3cret-agent-token";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tasks_run_their_tool_over_one_agent_snapshot() {
    let end_to_end = EndToEnd::start().await;
    let client = &end_to_end.client;
    let origin = Origin::new();
    origin.commit(&[
        (
            ".igloo/pipeline.toml",
            "[warm]\ncommand = \"echo warmed > warm.txt\"\nlockfiles = [\"deps.lock\"]\n\n\
             [[checks]]\nname = \"ok\"\nrun = \"true\"\n",
        ),
        (
            ".igloo/agents.toml",
            "default_tool = \"scripted\"\n\n[tools.scripted]\nharness = \"command\"\n\
             install = \"echo installed > tool.txt\"\ncommand = \"sh agent.sh\"\n\
             secrets = [\"AGENT_TOKEN\"]\n\n[tools.idle]\nharness = \"command\"\ncommand = \"echo\"\n",
        ),
        (".gitignore", "warm.txt\ntool.txt\n"),
        ("deps.lock", "v1"),
        (
            "agent.sh",
            "test -f warm.txt && test -f tool.txt && echo \"asked: $1\" && \
             echo \"token: $AGENT_TOKEN\" && echo \"$1\" > answer.txt\n",
        ),
    ]);
    let repo = client
        .register_repo(&RegisterRepoRequest::new(origin.path.clone()))
        .await
        .expect("register");
    client
        .set_secret(&repo.id, "AGENT_TOKEN", SECRET)
        .await
        .expect("secret");

    // Two tasks at once: each runs its first turn with its goal.
    let first = client
        .create_task(&repo.id, &CreateTaskRequest::new("Fix the bug"))
        .await
        .expect("create");
    let second = client
        .create_task(&repo.id, &CreateTaskRequest::new("Add a test"))
        .await
        .expect("create");
    for (task, goal) in [(&first, "Fix the bug"), (&second, "Add a test")] {
        let task = settled(client, &task.id).await;
        assert_eq!(task.phase, TaskPhase::AwaitingReview, "{task:?}");
        assert_eq!(task.commit.as_deref(), Some(origin.head("main").as_str()));
        assert_eq!(task.turns.len(), 1);
        assert_eq!(task.turns[0].status, TurnStatus::Succeeded);
        revised(client, &origin, &task, goal).await;
        let transcript = client
            .get_transcript(&task.id, 0)
            .await
            .expect("transcript");
        assert!(transcript.idle);
        assert_eq!(transcript.next, 2);
        let lines: Vec<&TranscriptItem> =
            transcript.entries.iter().map(|entry| &entry.item).collect();
        assert_eq!(
            lines,
            [
                &TranscriptItem::Output {
                    text: format!("asked: {goal}")
                },
                &TranscriptItem::Output {
                    text: "token: ***".to_owned()
                },
            ],
            "the tool's output, with its secret masked"
        );
        let rest = client
            .get_transcript(&task.id, 1)
            .await
            .expect("transcript");
        assert_eq!(rest.entries.len(), 1);
        assert_eq!(rest.entries[0].position, 1);
    }

    // The warm snapshot and the agent snapshot were each built once.
    let builds: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE type = 'igloo.build.requested'")
            .fetch_one(end_to_end.server.database().pool())
            .await
            .expect("count");
    assert_eq!(builds, 2, "one warm and one agent snapshot for both tasks");

    // A cancelled task stops its sandbox.
    let cancelled = client.cancel_task(&first.id).await.expect("cancel");
    assert_eq!(cancelled.phase, TaskPhase::Cancelled);
    let sandbox = cancelled.sandbox.expect("sandbox");
    stopped(client, &sandbox).await;

    // A task naming a tool the repository lacks fails, as does a turn without commits.
    for (tool, error) in [
        ("missing", "the repository has no tool missing"),
        ("idle", "turn 1 made no commits"),
    ] {
        let task = client
            .create_task(
                &repo.id,
                &CreateTaskRequest::new("Anything").with_tool(tool),
            )
            .await
            .expect("create");
        let task = settled(client, &task.id).await;
        assert_eq!(task.phase, TaskPhase::Failed);
        assert_eq!(task.error.as_deref(), Some(error));
    }
    end_to_end.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_review_sends_the_task_back_and_a_merge_finishes_it() {
    let end_to_end = EndToEnd::start().await;
    let client = &end_to_end.client;
    let origin = Origin::new();
    origin.commit(&[
        (
            ".igloo/pipeline.toml",
            "[[checks]]\nname = \"ok\"\nrun = \"true\"\n",
        ),
        (
            ".igloo/agents.toml",
            "default_tool = \"scripted\"\n\n[tools.scripted]\nharness = \"command\"\n\
             command = \"sh agent.sh\"\n",
        ),
        ("agent.sh", "printf '%s' \"$1\" > answer.txt\n"),
    ]);
    let repo = client
        .register_repo(&RegisterRepoRequest::new(origin.path.clone()))
        .await
        .expect("register");
    let task = client
        .create_task(&repo.id, &CreateTaskRequest::new("Write an answer"))
        .await
        .expect("create");
    let task = settled(client, &task.id).await;
    let change = task.change.clone().expect("change");
    assert_eq!(ended_run(client, &change, 1).await.phase, RunPhase::Passed);

    // A comment and a request for changes start a turn whose prompt holds the comment.
    client
        .comment_change(
            &change,
            &CommentRequest::new("Say why").on_path("answer.txt", Some(1)),
        )
        .await
        .expect("comment");
    client.request_changes(&change).await.expect("request");
    let task = second_revision(client, &task.id).await;
    assert_eq!(task.phase, TaskPhase::AwaitingReview, "{task:?}");
    let prompt = &task.turns[1].prompt;
    assert!(prompt.contains("- answer.txt:1: Say why"), "{prompt}");
    let head = client.get_change(&change).await.expect("change").revisions[1]
        .head
        .clone();
    assert_eq!(
        origin.git(&[
            "--git-dir",
            &origin.path,
            "show",
            &format!("{head}:answer.txt")
        ]),
        prompt.trim(),
        "the second turn answered the review"
    );

    // A human approves and merges; the task is done and its sandbox stops.
    assert_eq!(ended_run(client, &change, 2).await.phase, RunPhase::Passed);
    client
        .approve_change(&change)
        .await
        .expect("a human approves");
    client.merge_change(&change).await.expect("merge");
    let main = origin.head("main");
    let tree = |commit: &str| {
        origin.git(&[
            "-C",
            &origin.path,
            "rev-parse",
            &format!("{commit}^{{tree}}"),
        ])
    };
    assert_eq!(tree(&main), tree(&head), "the squash holds the task's work");
    let done = settled_end(client, &task.id).await;
    assert_eq!(done.phase, TaskPhase::Done);
    stopped(client, &done.sandbox.expect("sandbox")).await;
    end_to_end.stop().await;
}

/// Task `id` once its second turn's commits are a revision, within a minute.
async fn second_revision(client: &Client, id: &str) -> TaskResource {
    for _ in 0..600 {
        let task = client.get_task(id).await.expect("task");
        if task.turns.len() == 2 && task.phase != TaskPhase::Working {
            return task;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("task {id} took no second turn");
}

/// Task `id` once it ended, within ten seconds.
async fn settled_end(client: &Client, id: &str) -> TaskResource {
    for _ in 0..100 {
        let task = client.get_task(id).await.expect("task");
        if matches!(
            task.phase,
            TaskPhase::Done | TaskPhase::Failed | TaskPhase::Cancelled
        ) {
            return task;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("task {id} did not end");
}

/// Checks `task`'s change: one revision, the head of the branch Igloo pushed, holding the
/// tool's work committed over the task's commit, with its checks passed.
async fn revised(client: &Client, origin: &Origin, task: &TaskResource, goal: &str) {
    let change = client
        .get_change(task.change.as_deref().expect("change"))
        .await
        .expect("change");
    let branch = format!("igloo/tasks/{}", task.id);
    assert_eq!(change.source_branch, branch);
    assert_eq!(change.title, goal);
    assert_eq!(change.revisions.len(), 1);
    let head = origin.head(&branch);
    assert_eq!(change.revisions[0].head, head);
    assert_eq!(
        origin.git(&[
            "--git-dir",
            &origin.path,
            "show",
            &format!("{head}:answer.txt")
        ]),
        goal,
        "the tool's uncommitted work was committed"
    );
    assert_eq!(
        origin.git(&["--git-dir", &origin.path, "rev-parse", &format!("{head}^")]),
        task.commit.clone().expect("commit")
    );
    let run = ended_run(client, &change.id, 1).await;
    assert_eq!(run.phase, RunPhase::Passed, "{run:?}");
}

/// Task `id` once it is no longer preparing or working, within a minute.
async fn settled(client: &Client, id: &str) -> TaskResource {
    for _ in 0..600 {
        let task = client.get_task(id).await.expect("task");
        if !matches!(task.phase, TaskPhase::Preparing | TaskPhase::Working) {
            return task;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("task {id} did not settle");
}

/// Waits up to ten seconds for sandbox `id` to be asked to stop.
async fn stopped(client: &Client, id: &str) {
    for _ in 0..100 {
        let sandbox = client.get_sandbox(id).await.expect("sandbox");
        if sandbox.desired == DesiredState::Stopped {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the cancelled task's sandbox did not stop");
}
