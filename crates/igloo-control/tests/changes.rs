//! The phase 3 exit gate against a git repository on disk: a change from a branch, checks on
//! every revision with a secret, a merge Igloo pushes, approval of protected paths, and the
//! outcome a merge records, including its revert.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "a test reports failures by panicking"
)]

use igloo::change::{ChangePhase, OpenChangeRequest};
use igloo::ci::{CheckStatus, RunPhase};
use igloo::repo::RegisterRepoRequest;
use serde_json::Value;

use self::common::{EndToEnd, Origin, ended_run, refusal};

mod common;

const SECRET: &str = "s3cret-deploy-token";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changes_are_checked_merged_and_their_outcomes_recorded() {
    let end_to_end = EndToEnd::start().await;
    let client = &end_to_end.client;
    let origin = Origin::new();
    let pipeline = "secrets = [\"DEPLOY_TOKEN\"]\n\n[[checks]]\nname = \"secret\"\n\
                    run = \"test -n \\\"$DEPLOY_TOKEN\\\" && echo token: $DEPLOY_TOKEN\"\n\n\
                    [[checks]]\nname = \"ready\"\nrun = \"test -f ready\"\n";
    origin.commit(&[
        (".igloo/pipeline.toml", pipeline),
        (".igloo/agents.toml", "protected = [\"api/**\"]\n"),
    ]);
    origin.branch("feature");
    origin.commit(&[("api/schema.json", "{}")]);
    let repo = client
        .register_repo(&RegisterRepoRequest::new(origin.path.clone()))
        .await
        .expect("register");
    client
        .set_secret(&repo.id, "DEPLOY_TOKEN", SECRET)
        .await
        .expect("secret");

    // 1. A change from a branch; its first revision is the branch's head over main.
    let change = client
        .open_change(&repo.id, &OpenChangeRequest::new("feature", "Ship the API"))
        .await
        .expect("open");
    assert_eq!(change.revisions[0].head, origin.head("feature"));

    // 2. Checks run on every revision, with the repository's secret.
    let first = ended_run(client, &change.id, 1).await;
    assert_eq!(first.phase, RunPhase::Failed);
    assert_eq!(
        first.checks[0].status,
        CheckStatus::Passed,
        "the secret reached the check"
    );
    assert_eq!(first.checks[1].status, CheckStatus::Failed);
    origin.commit(&[("ready", "")]);
    client.revise_change(&change.id).await.expect("revise");
    let second = ended_run(client, &change.id, 2).await;
    assert_eq!(second.phase, RunPhase::Passed, "{second:?}");

    // 5. The secret never appears in logs, events or responses.
    let job = second.checks[0].job.clone().expect("job");
    let logs = end_to_end.logs(&job).await;
    assert!(logs.contains("token: ***"), "{logs}");
    assert!(!logs.contains(SECRET));
    let events: Vec<String> = sqlx::query_scalar("SELECT data::text FROM events")
        .fetch_all(end_to_end.server.database().pool())
        .await
        .expect("events");
    assert!(events.iter().all(|event| !event.contains(SECRET)));

    // 3. Protected paths need a human approval; Igloo then pushes main.
    assert_eq!(
        refusal(client, &change.id).await,
        "change.approval_required"
    );
    client.approve_change(&change.id).await.expect("approve");
    let merged = client.merge_change(&change.id).await.expect("merge");
    assert_eq!(merged.phase, ChangePhase::Merged);
    // One squashed commit on the old main, holding the revision's tree and naming the change.
    let main = origin.head("main");
    origin.git(&["fetch", "--quiet", &origin.path, "main"]);
    assert_eq!(
        origin.git(&["rev-parse", &format!("{main}^{{tree}}")]),
        origin.git(&["rev-parse", "feature^{tree}"])
    );
    assert_eq!(
        origin.git(&["rev-list", "--count", &format!("{main}^..{main}")]),
        "1"
    );
    assert_eq!(
        origin.git(&["merge-base", &main, "feature"]),
        origin.git(&["rev-parse", &format!("{main}^")]),
        "the squash sits on the main the checks were judged against"
    );
    assert!(
        origin
            .git(&["log", "-1", "--format=%B", &main])
            .ends_with(&format!("Igloo-Change: {}", change.id))
    );
    let shown = serde_json::to_string(&merged).expect("json");
    assert!(!shown.contains(SECRET));

    // 4. The merge records its outcome.
    let outcome = recorded(&end_to_end, "igloo.outcome.recorded").await;
    let record = &outcome["record"];
    assert_eq!(record["change"], change.id);
    assert_eq!(record["revision"], 2);
    assert_eq!(record["verdict"]["commit"], origin.head("main"));
    assert_eq!(record["checks"].as_array().map(Vec::len), Some(2));
    assert_eq!(record["approvals"].as_array().map(Vec::len), Some(1));
    assert_eq!(record["commits"], serde_json::json!([origin.head("main")]));

    // A revert of the merged change on main is recorded against it.
    origin.git(&["checkout", "--quiet", "main"]);
    origin.git(&["reset", "--quiet", "--hard", &origin.head("main")]);
    origin.git(&[
        "-c",
        "user.name=igloo",
        "-c",
        "user.email=igloo@example.com",
        "revert",
        "--no-edit",
        "--quiet",
        &origin.head("main"),
    ]);
    origin.git(&["push", "--quiet", &origin.path, "main"]);
    origin.git(&["checkout", "--quiet", "-b", "next"]);
    origin.commit(&[("next.txt", "next")]);
    client
        .open_change(&repo.id, &OpenChangeRequest::new("next", "Next thing"))
        .await
        .expect("open another change");
    let reverted = recorded(&end_to_end, "igloo.outcome.reverted").await;
    assert_eq!(
        reverted["commit"],
        origin.git(&["rev-parse", "main"]),
        "the revert is recorded against the merged change"
    );
    end_to_end.stop().await;
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
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("no {kind} event");
}
