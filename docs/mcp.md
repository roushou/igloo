# Igloo over MCP

Igloo serves MCP over streamable HTTP at `/mcp`, on the same address as the REST API, with the
same bearer token. Its tools create and follow tasks and review their changes; approving and
merging stay with a human, through the CLI.

| Tool                     | Does                                                             |
| ------------------------ | ---------------------------------------------------------------- |
| `repo.list`              | Lists the registered repositories.                               |
| `task.create`            | Creates a task on a repository, with its default tool or another |
| `task.get`, `task.list`  | A task: phase, turns, change, error; a repository's tasks        |
| `task.transcript`        | What the task's agent said and did, from a position on           |
| `task.cancel`            | Cancels a task and stops its sandbox                             |
| `change.get`             | A change with its revisions, comments, approvals and check runs  |
| `change.comment`         | Comments on a revision, optionally on a line of a file           |
| `change.request_changes` | Sends the comments since the last request to the task's agent    |

## Claude Code

```sh
claude mcp add --transport http igloo http://<server>:7000/mcp \
  --header "Authorization: Bearer $IGLOO_TOKEN"
```

## Codex

In `~/.codex/config.toml`:

```toml
[mcp_servers.igloo]
url = "http://<server>:7000/mcp"
bearer_token_env_var = "IGLOO_TOKEN"
```

## Other clients

Any MCP client speaking streamable HTTP works: point it at `http://<server>:7000/mcp` and send
`Authorization: Bearer <token>`. Clients without MCP use the `igloo` CLI, which offers the same
operations.
