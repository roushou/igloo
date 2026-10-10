//! Igloo's MCP server, driven over streamable HTTP as an editor's agent drives it: the tools
//! are listed, a task is created and followed to its change.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "a test reports failures by panicking"
)]

use std::time::Duration;

use igloo::repo::RegisterRepoRequest;
use serde_json::{Value, json};

use self::common::{DEV_TOKEN, EndToEnd, McpSession, Origin};

mod common;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_editor_agent_creates_a_task_and_reads_its_change_over_mcp() {
    let end_to_end = EndToEnd::start().await;
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
        ("agent.sh", "echo \"$1\" > answer.txt\n"),
    ]);
    let repo = end_to_end
        .client
        .register_repo(&RegisterRepoRequest::new(origin.path.clone()))
        .await
        .expect("register");
    let url = format!("http://{}/mcp", end_to_end.server.rest_address());

    let unauthorized = reqwest::Client::builder()
        .build()
        .expect("http")
        .post(&url)
        .body("{}")
        .send()
        .await
        .expect("send");
    assert_eq!(unauthorized.status(), 401, "MCP needs the bearer token");

    let mut session = McpSession::open(url, DEV_TOKEN).await;
    let tools = session.request("tools/list", json!({})).await;
    let mut names: Vec<&str> = tools["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "change.comment",
            "change.get",
            "change.request_changes",
            "repo.list",
            "task.cancel",
            "task.create",
            "task.get",
            "task.hand_back",
            "task.list",
            "task.take_over",
            "task.transcript",
        ],
        "no tool approves or merges"
    );

    let repos = session.call("repo.list", json!({})).await;
    assert_eq!(repos[0]["id"], repo.id);
    let task = session
        .call(
            "task.create",
            json!({ "repo": repo.id, "goal": "Write an answer" }),
        )
        .await;
    let id = task["id"].as_str().expect("id").to_owned();
    let mut found = Value::Null;
    for _ in 0..600 {
        found = session.call("task.get", json!({ "id": id })).await;
        if found["phase"] != "preparing" && found["phase"] != "working" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(found["phase"], "awaiting_review", "{found}");
    let change = session
        .call("change.get", json!({ "id": found["change"] }))
        .await;
    assert_eq!(change["change"]["title"], "Write an answer");
    assert_eq!(change["runs"].as_array().map(Vec::len), Some(1));

    let refused = session
        .request(
            "tools/call",
            json!({ "name": "change.request_changes", "arguments": { "id": found["change"] } }),
        )
        .await;
    assert_eq!(refused["isError"], true);
    assert_eq!(
        refused["structuredContent"]["code"],
        "change.nothing_to_request"
    );
    end_to_end.stop().await;
}
