//! The phase 4 exit gate against a git repository on disk, with a scripted tool: tasks created
//! over MCP, their transcripts, the changes their commits become, a review sending one back,
//! a human merge and the outcome it records, and a credential that never shows.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "a test reports failures by panicking"
)]

use std::time::Duration;

use igloo::repo::RegisterRepoRequest;
use serde_json::{Value, json};

use self::common::{DEV_TOKEN, EndToEnd, McpSession, Origin, ended_run, refusal};

mod common;

const SECRET: &str = "s3cret-agent-credential";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agents_work_on_tasks_from_the_editor_and_a_human_merges() {
    let end_to_end = EndToEnd::start().await;
    let client = &end_to_end.client;
    let origin = Origin::new();
    origin.commit(&[
        (".igloo/pipeline.toml", "[[checks]]\nname = \"ok\"\nrun = \"true\"\n"),
        (
            ".igloo/agents.toml",
            "protected = [\"api/**\"]\ndefault_tool = \"scripted\"\n\n[tools.scripted]\n\
             harness = \"command\"\ncommand = \"sh agent.sh\"\nsecrets = [\"AGENT_CREDENTIAL\"]\n",
        ),
        (
            "agent.sh",
            "echo \"credential: $AGENT_CREDENTIAL\"\nmkdir -p api\nprintf '%s' \"$1\" > api/answer.txt\n",
        ),
    ]);
    let repo = client
        .register_repo(&RegisterRepoRequest::new(origin.path.clone()))
        .await
        .expect("register");
    client
        .set_secret(&repo.id, "AGENT_CREDENTIAL", SECRET)
        .await
        .expect("secret");
    let url = format!("http://{}/mcp", end_to_end.server.rest_address());
    let mut mcp = McpSession::open(url, DEV_TOKEN).await;

    // 1. Two tasks at once, from the editor.
    let mut tasks = Vec::new();
    for goal in ["Answer one", "Answer two"] {
        let task = mcp
            .call("task.create", json!({ "repo": repo.id, "goal": goal }))
            .await;
        tasks.push(task["id"].as_str().expect("id").to_owned());
    }
    for task in &tasks {
        let found = settled(&mut mcp, task, 1).await;
        assert_eq!(found["phase"], "awaiting_review", "{found}");

        // 2. The transcript holds what the tool said, its credential masked.
        let transcript = mcp.call("task.transcript", json!({ "id": task })).await;
        assert_eq!(
            transcript["entries"][0]["item"],
            json!({ "kind": "output", "text": "credential: ***" })
        );

        // 3. The commits are a revision of the task's change, and its checks run.
        let change = found["change"].as_str().expect("change");
        assert_eq!(
            ended_run(client, change, 1).await.phase,
            igloo::ci::RunPhase::Passed
        );
    }

    // 4. A review sends the first task back to its agent for a new revision.
    let task = &tasks[0];
    let change = settled(&mut mcp, task, 1).await["change"]
        .as_str()
        .expect("change")
        .to_owned();
    mcp.call(
        "change.comment",
        json!({ "id": change, "body": "Say why", "path": "api/answer.txt", "line": 1 }),
    )
    .await;
    mcp.call("change.request_changes", json!({ "id": change }))
        .await;
    let revised = settled(&mut mcp, task, 2).await;
    assert_eq!(revised["phase"], "awaiting_review", "{revised}");
    assert!(
        revised["turns"][1]["prompt"]
            .as_str()
            .is_some_and(|prompt| prompt.contains("api/answer.txt:1: Say why"))
    );
    ended_run(client, &change, 2).await;

    // 5. The change touches a protected path: a human approves, then Igloo merges.
    assert_eq!(refusal(client, &change).await, "change.approval_required");
    client.approve_change(&change).await.expect("approve");
    client.merge_change(&change).await.expect("merge");
    let mut phase = Value::Null;
    for _ in 0..100 {
        phase = mcp.call("task.get", json!({ "id": task })).await["phase"].clone();
        if phase == "done" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(phase, "done", "a merged change finishes its task");

    // 6. The outcome names the task, its tool and its turns.
    let work = recorded(&end_to_end, "igloo.outcome.attributed").await;
    assert_eq!(
        work["work"],
        json!({ "task": task, "tool": "scripted", "turns": 2 })
    );

    // 7. The credential never appears in events or responses.
    let events: Vec<String> = sqlx::query_scalar("SELECT data::text FROM events")
        .fetch_all(end_to_end.server.database().pool())
        .await
        .expect("events");
    assert!(events.iter().all(|event| !event.contains(SECRET)));
    for task in &tasks {
        let shown = mcp.call("task.get", json!({ "id": task })).await;
        assert!(!shown.to_string().contains(SECRET));
    }
    end_to_end.stop().await;
}

/// Task `id` once it has `turns` turns and is neither preparing nor working, within a minute.
async fn settled(mcp: &mut McpSession, id: &str, turns: usize) -> Value {
    for _ in 0..600 {
        let task = mcp.call("task.get", json!({ "id": id })).await;
        let busy = task["phase"] == "preparing" || task["phase"] == "working";
        if !busy && task["turns"].as_array().map_or(0, Vec::len) >= turns {
            return task;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("task {id} did not settle after {turns} turns");
}

/// The data of the first event of `kind`, within ten seconds.
async fn recorded(end_to_end: &EndToEnd, kind: &str) -> Value {
    for _ in 0..100 {
        let found: Option<Value> =
            sqlx::query_scalar("SELECT data FROM events WHERE type = $1 ORDER BY sequence LIMIT 1")
                .bind(kind)
                .fetch_optional(end_to_end.server.database().pool())
                .await
                .expect("events");
        if let Some(data) = found {
            return data;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no {kind} event");
}
