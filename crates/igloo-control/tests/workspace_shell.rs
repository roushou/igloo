//! A workspace is a shell with its checkout wired to Igloo: its terminal runs commands, its
//! `origin` is Igloo's git endpoint with a credential that works from inside the sandbox
//! though the API token never enters it, and a push from it opens a change.
//!
//! Needs Docker for Postgres, like the other end-to-end tests; the workspace runs on the
//! embedded worker's process runtime.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "a test reports failures by panicking"
)]

use std::time::Duration;

use igloo::repo::RegisterRepoRequest;
use igloo::workspace::{CreateWorkspaceRequest, WorkspacePhase};
use igloo::{Client, TerminalEvent, TerminalExit};

use self::common::{DEV_TOKEN, EndToEnd, Origin};

mod common;

/// Runs `script` in a terminal of `sandbox`; what it printed and its exit code.
async fn shell(client: &Client, sandbox: &str, script: &str) -> (String, i32) {
    let command = ["sh".to_owned(), "-c".to_owned(), script.to_owned()];
    let terminal = client
        .open_terminal(sandbox, &command, 120, 40)
        .await
        .expect("terminal");
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_workspace_pushes_through_igloo_without_the_api_token() {
    let end_to_end = EndToEnd::start().await;
    let client = &end_to_end.client;
    let origin = Origin::new();
    origin.commit(&[("README.md", "igloo")]);
    let repo = client
        .register_repo(&RegisterRepoRequest::new(origin.path.clone()))
        .await
        .expect("register");

    let workspace = client
        .create_workspace(&repo.id, &CreateWorkspaceRequest::new())
        .await
        .expect("create");
    let running = client
        .start_workspace_and_wait(&workspace.id, Duration::from_secs(120))
        .await
        .expect("the workspace runs");
    assert_eq!(running.phase, WorkspacePhase::Running);
    let sandbox = running.sandbox.expect("sandbox");

    // The terminal runs commands in the checkout and reports their exit code.
    let (printed, code) = shell(client, &sandbox, "cat README.md; exit 3").await;
    assert_eq!(code, 3);
    assert!(printed.contains("igloo"), "{printed}");

    // The setup job points origin at Igloo's git endpoint, once the sandbox runs.
    let mut set_up = false;
    for _ in 0..300 {
        let (_, code) = shell(client, &sandbox, "test -e .git/igloo-workspace-setup").await;
        if code == 0 {
            set_up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(set_up, "the workspace was never set up");
    let origin_url = format!(
        "http://{}{}",
        end_to_end.server.rest_address(),
        repo.git_path
    );
    let (printed, _) = shell(client, &sandbox, "git remote get-url origin").await;
    assert_eq!(printed.trim(), origin_url);

    // The API token is not in the sandbox; a narrower credential is.
    let (environment, _) = shell(client, &sandbox, "env").await;
    assert!(!environment.contains(DEV_TOKEN), "{environment}");
    assert!(environment.contains("GIT_CONFIG_VALUE_0=Authorization: Bearer iglws."));

    // A push from the workspace opens a change on the default branch.
    let push = "git checkout -q -b feature \
                && echo hello > hello.txt && git add hello.txt \
                && git -c user.name=Test -c user.email=test@igloo.invalid commit -q -m 'Add hello' \
                && git push -q origin feature";
    let (printed, code) = shell(client, &sandbox, push).await;
    assert_eq!(code, 0, "{printed}");
    let changes = client.list_changes(&repo.id).await.expect("changes");
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].source_branch, "feature");
    assert_eq!(changes[0].title, "Add hello");

    // The default branch still moves only by merge.
    let (printed, code) = shell(client, &sandbox, "git push origin HEAD:main 2>&1; exit $?").await;
    assert_ne!(code, 0, "{printed}");
    assert!(printed.contains("igloo change merge"), "{printed}");

    end_to_end.stop().await;
}
