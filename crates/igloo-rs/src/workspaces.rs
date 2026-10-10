use std::time::Duration;

use igloo_api::workspace::{WorkspacePhase, WorkspaceResource};

use crate::client::{Client, Error};

impl Client {
    /// How often a starting workspace is looked at.
    const WORKSPACE_POLL: Duration = Duration::from_millis(500);

    /// The caller's most recently used workspace of `repo`, if it has one. A workspace counts as
    /// used when a terminal attached to it or when it started; ties go to the newer.
    pub async fn latest_workspace(&self, repo: &str) -> Result<Option<WorkspaceResource>, Error> {
        Ok(self
            .list_workspaces()
            .await?
            .into_iter()
            .filter(|workspace| workspace.repo == repo)
            .max_by_key(|workspace| (workspace.last_activity, workspace.created_at)))
    }

    /// Starts workspace `id` unless it runs, and waits up to `timeout` for its sandbox to run:
    /// a stopped workspace resumes from what its last stop sealed, a stopping one starts again
    /// once it has stopped. Returns the running workspace; its `sandbox` is set.
    pub async fn start_workspace_and_wait(
        &self,
        id: &str,
        timeout: Duration,
    ) -> Result<WorkspaceResource, Error> {
        let mut workspace = self.start_workspace(id).await?;
        let waited = tokio::time::timeout(timeout, async {
            loop {
                match (workspace.phase, &workspace.sandbox) {
                    (WorkspacePhase::Running, Some(_)) => return Ok(workspace),
                    (WorkspacePhase::Stopped, _) => {
                        return Err(Error::Workspace(format!(
                            "{id} stopped before it ran; start it again or check its sandbox"
                        )));
                    }
                    _ => {}
                }
                tokio::time::sleep(Self::WORKSPACE_POLL).await;
                workspace = self.get_workspace(id).await?;
            }
        })
        .await;
        waited.unwrap_or_else(|_| {
            Err(Error::Workspace(format!(
                "{id} did not run within {} seconds",
                timeout.as_secs()
            )))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    use super::*;

    /// Runs `scenario` against a server that answers each request, in order, with the next
    /// scripted `(status, body)`; returns the request lines the server saw.
    async fn scripted<C: Future<Output = ()>>(
        script: Vec<(u16, String)>,
        scenario: impl FnOnce(Client) -> C,
    ) -> Vec<String> {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&seen);
        let server = async move {
            let mut script = script.into_iter();
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut request = vec![0_u8; 8192];
                let read = stream.read(&mut request).await.expect("read");
                let text = String::from_utf8_lossy(&request[..read]).into_owned();
                let line = text.lines().next().unwrap_or_default().to_owned();
                recorded.lock().expect("lock").push(line);
                let (status, body) = script.next().expect("a scripted response");
                let response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.expect("write");
            }
        };
        let client = Client::new(&format!("http://{address}"), "t").expect("client");
        tokio::select! {
            () = server => {}
            () = scenario(client) => {}
        }
        let mut seen = seen.lock().expect("lock");
        std::mem::take(&mut *seen)
    }

    fn workspace(id: &str, repo: &str, phase: &str, active: &str) -> serde_json::Value {
        let mut workspace = json!({
            "id": id, "owner": "usr_1", "repo": repo, "branch": "main", "phase": phase,
            "last_activity": active, "created_at": "2026-10-01T00:00:00Z",
        });
        if phase == "running" {
            workspace["sandbox"] = json!("sbx_1");
        }
        workspace
    }

    #[tokio::test]
    async fn the_latest_workspace_is_the_repositorys_most_recently_used() {
        let list = json!([
            workspace("wsp_a", "repo_1", "stopped", "2026-10-03T00:00:00Z"),
            workspace("wsp_b", "repo_1", "running", "2026-10-05T00:00:00Z"),
            workspace("wsp_c", "repo_2", "running", "2026-10-09T00:00:00Z"),
        ])
        .to_string();
        scripted(
            vec![(200, list.clone()), (200, list)],
            |client| async move {
                let latest = client.latest_workspace("repo_1").await.expect("list");
                assert_eq!(latest.map(|workspace| workspace.id), Some("wsp_b".into()));
                let none = client.latest_workspace("repo_3").await.expect("list");
                assert!(none.is_none());
            },
        )
        .await;
    }

    #[tokio::test]
    async fn a_stopped_workspace_is_started_and_waited_for() {
        let at = "2026-10-05T00:00:00Z";
        let script = vec![
            (
                200,
                workspace("wsp_a", "repo_1", "starting", at).to_string(),
            ),
            (
                200,
                workspace("wsp_a", "repo_1", "starting", at).to_string(),
            ),
            (200, workspace("wsp_a", "repo_1", "running", at).to_string()),
        ];
        let seen = scripted(script, |client| async move {
            let workspace = client
                .start_workspace_and_wait("wsp_a", Duration::from_secs(60))
                .await
                .expect("running");
            assert_eq!(workspace.sandbox.as_deref(), Some("sbx_1"));
        })
        .await;
        assert_eq!(
            seen,
            [
                "POST /v1/workspaces/wsp_a/start HTTP/1.1",
                "GET /v1/workspaces/wsp_a HTTP/1.1",
                "GET /v1/workspaces/wsp_a HTTP/1.1",
            ]
        );
    }

    #[tokio::test]
    async fn a_running_workspace_returns_at_once() {
        let at = "2026-10-05T00:00:00Z";
        let script = vec![(200, workspace("wsp_a", "repo_1", "running", at).to_string())];
        let seen = scripted(script, |client| async move {
            client
                .start_workspace_and_wait("wsp_a", Duration::from_secs(60))
                .await
                .expect("running");
        })
        .await;
        assert_eq!(seen.len(), 1);
    }

    #[tokio::test]
    async fn a_workspace_that_stops_while_starting_or_never_runs_is_an_error() {
        let at = "2026-10-05T00:00:00Z";
        let script = vec![
            (
                200,
                workspace("wsp_a", "repo_1", "starting", at).to_string(),
            ),
            (200, workspace("wsp_a", "repo_1", "stopped", at).to_string()),
        ];
        scripted(script, |client| async move {
            let error = client
                .start_workspace_and_wait("wsp_a", Duration::from_secs(60))
                .await
                .expect_err("stopped");
            assert!(matches!(&error, Error::Workspace(message) if message.contains("stopped")));
        })
        .await;

        let starting = workspace("wsp_a", "repo_1", "starting", at).to_string();
        scripted(vec![(200, starting); 8], |client| async move {
            let error = client
                .start_workspace_and_wait("wsp_a", Duration::from_secs(1))
                .await
                .expect_err("timeout");
            assert!(matches!(&error, Error::Workspace(message) if message.contains("did not run")));
        })
        .await;
    }
}
