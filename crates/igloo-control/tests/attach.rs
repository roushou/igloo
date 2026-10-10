//! Attaching to an agent: a person watches a task's sandbox read-only, takes the task over, works
//! in the sandbox while the task waits, and hands it back with a turn that names what they changed.
//!
//! Needs Docker for Postgres, like the other end-to-end tests; the sandbox runs on the embedded
//! worker's process runtime, with `git` on the host.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "a test reports failures by panicking"
)]

use std::time::Duration;

use igloo::change::CommentRequest;
use igloo::repo::RegisterRepoRequest;
use igloo::task::{CreateTaskRequest, TakeoverPhase, TaskPhase, TaskResource};
use igloo::{Client, Terminal, TerminalEvent, TerminalExit, TerminalMode};

use self::common::{EndToEnd, Origin};

mod common;

/// Runs `script` in a writable terminal of `sandbox`; what it printed and its exit code.
async fn shell(client: &Client, sandbox: &str, script: &str) -> (String, i32) {
    let command = ["sh".to_owned(), "-c".to_owned(), script.to_owned()];
    let terminal = client
        .open_terminal(sandbox, &command, 120, 40)
        .await
        .expect("a writable terminal");
    let (_input, mut output) = terminal.split();
    let mut printed = Vec::new();
    let collect = async {
        loop {
            match output.next().await.expect("terminal output") {
                Some(TerminalEvent::Output(bytes)) => printed.extend(bytes),
                Some(TerminalEvent::Exit(TerminalExit::Code(code))) => return code,
                other => panic!("the terminal ended unexpectedly: {other:?}"),
            }
        }
    };
    let code = tokio::time::timeout(Duration::from_secs(60), collect)
        .await
        .expect("the command finished in time");
    (String::from_utf8_lossy(&printed).into_owned(), code)
}

/// Reads a read-only terminal until it printed `needle`, within a minute.
async fn watched(terminal: Terminal, needle: &str) -> String {
    let (mut input, mut output) = terminal.split();
    // Whatever is typed into a read-only terminal is dropped by the server.
    input
        .send(b"touch typed-by-a-watcher\n".to_vec())
        .await
        .expect("typing is accepted by the socket and dropped by the server");
    let mut printed = String::new();
    let read = async {
        while let Some(event) = output.next().await.expect("terminal output") {
            if let TerminalEvent::Output(bytes) = event {
                printed.push_str(&String::from_utf8_lossy(&bytes));
                if printed.contains(needle) {
                    return;
                }
            }
        }
        panic!("the view ended before it printed {needle:?}: {printed}");
    };
    tokio::time::timeout(Duration::from_secs(60), read)
        .await
        .expect("the view printed in time");
    printed
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_person_watches_takes_over_and_hands_back_with_what_they_changed() {
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
    assert_eq!(task.phase, TaskPhase::AwaitingReview, "{task:?}");
    let sandbox = task.sandbox.clone().expect("sandbox");
    let change = task.change.clone().expect("change");

    // A read-only attach shows the sandbox and cannot write to it; a writable one is refused.
    let view = client
        .attach_terminal(&sandbox, TerminalMode::ReadOnly, 100, 30)
        .await
        .expect("a read-only terminal");
    let shown = watched(view, "Write an answer").await;
    assert!(
        shown.contains("answer.txt") || shown.contains("##"),
        "{shown}"
    );
    let refused = client
        .attach_terminal(&sandbox, TerminalMode::ReadWrite, 100, 30)
        .await
        .expect_err("not taken over");
    assert!(
        refused.to_string().contains("task.not_taken_over"),
        "{refused}"
    );

    // Taking over makes the terminal writable. The watcher's typing never reached the sandbox.
    let taken = client.take_over_task(&task.id).await.expect("take over");
    let takeover = taken.takeover.expect("taken over");
    assert_eq!(takeover.phase, TakeoverPhase::Paused);
    let (_, code) = shell(client, &sandbox, "test ! -e typed-by-a-watcher").await;
    assert_eq!(code, 0, "a read-only terminal wrote to the sandbox");
    let (_, code) = shell(
        client,
        &sandbox,
        "echo person > person.txt && git add person.txt \
         && git -c user.name=Person -c user.email=person@example.invalid commit -q -m 'Person commit' \
         && echo scratch > notes.txt",
    )
    .await;
    assert_eq!(code, 0);

    // The task waits: a review request does not start a turn while the person holds it.
    client
        .comment_change(&change, &CommentRequest::new("Say why"))
        .await
        .expect("comment");
    client.request_changes(&change).await.expect("request");
    tokio::time::sleep(Duration::from_secs(3)).await;
    let waiting = client.get_task(&task.id).await.expect("task");
    assert_eq!(waiting.turns.len(), 1, "no turn starts while taken over");
    assert_eq!(waiting.takeover.expect("held").phase, TakeoverPhase::Paused);

    // Handing back resumes it with a turn that names the person's commit and uncommitted file.
    client.hand_back_task(&task.id).await.expect("hand back");
    let resumed = second_revision(client, &task.id).await;
    assert!(resumed.takeover.is_none(), "{resumed:?}");
    let prompt = &resumed.turns[1].prompt;
    assert!(prompt.contains("Person commit"), "{prompt}");
    assert!(prompt.contains("person.txt"), "{prompt}");
    assert!(prompt.contains("notes.txt"), "{prompt}");
    assert!(
        prompt.contains("Say why"),
        "the pending review follows: {prompt}"
    );
    let head = client.get_change(&change).await.expect("change").revisions[1]
        .head
        .clone();
    assert_eq!(
        origin
            .git(&[
                "--git-dir",
                &origin.path,
                "show",
                &format!("{head}:person.txt")
            ])
            .trim(),
        "person",
        "the person's work is part of the next revision"
    );
    end_to_end.stop().await;
}

/// Task `id` once its second turn's commits are a revision, within a minute.
async fn second_revision(client: &Client, id: &str) -> TaskResource {
    for _ in 0..600 {
        let task = client.get_task(id).await.expect("task");
        if task.turns.len() == 2 && task.phase == TaskPhase::AwaitingReview {
            return task;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("task {id} took no second turn");
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
