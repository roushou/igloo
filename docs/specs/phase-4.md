# Phase 4: Agents

Exit gate, for the Igloo repository itself:

1. From the agent in the user's editor, over MCP, the user creates two tasks on Igloo at once.
   Each runs a coding tool headless in its own sandbox forked from the warm snapshot at the
   head of `main`.
2. Each task's transcript streams while it works: messages, tool calls, commands and their
   results. `igloo task show <id>` and the MCP tools show it.
3. When the agent's turn ends, its commits become a revision of the task's change, and the
   checks of `.igloo/pipeline.toml` run on it as in phase 3.
4. A review comment on the change sends the task back to its agent, with the comment, for a
   new revision.
5. A human approves and merges the change as in phase 3. An agent never approves; protected
   paths still need a human.
6. The outcome of the merged change names its task, tool and number of turns.
7. The tool's credential is a repository secret and never appears in events, logs, transcripts
   or responses.

Order: P4.1 -> P4.2 and P4.3 in parallel -> P4.4 -> P4.5 -> P4.6 -> P4.7.

## P4.1 Agent configuration

- Contract changes: `schemas/agents.schema.json` (new)
- Read first: ADR 0011, architecture §12

### Deliverables

- `.igloo/agents.toml` gains, beside `protected`:
  - `default_tool`: the tool tasks use unless they name another.
  - `[tools.<name>]`: `harness` (`claude-code`, `codex`, `command`), `install` (a command run
    once into the agent snapshot), `secrets` (names), `timeout_seconds` per turn,
    `max_turns` per task.
- `AgentSettings` parsed with the `toml` crate, read from the default branch like `TrustSettings`,
  with a JSON Schema in `schemas/`.
- Agent snapshot keys: the key of what the tool's `install` runs over (the warm snapshot's)
  plus the command. The snapshots are built and recorded on the repository's warm map when a
  task needs one (P4.2).

### Acceptance

- Igloo's own `.igloo/agents.toml` parses; the schema names every field.
- Equal inputs give equal agent snapshot keys; another install command or base gives another.

## P4.2 Tasks

- Contract changes: `igloo-api`, `schemas/openapi.json`, migrations (none expected)

### Deliverables

- `Build` resource (platform): a command run in a sandbox over a snapshot, sealed, and recorded
  on the repository under a key. A build of the same repository and key that is in progress or
  built is reused. Runs build their warm snapshots through it.
- `Task` resource (agents module, `agents/`): repo, goal, tool, commit, sandbox, change, turns,
  phase `Preparing | Working | AwaitingReview | Done | Failed | Cancelled`.
- `Agent` controller: reads the tool and pipeline at the default branch's head, builds the warm
  and agent snapshots it lacks, checks the commit out over the agent snapshot, creates the
  sandbox (network allowed), and submits each turn as a job with the prompt in `IGLOO_PROMPT`.
- `POST /v1/repos/{id}/tasks`, `GET /v1/tasks/{id}`, `GET /v1/repos/{id}/tasks`,
  `POST /v1/tasks/{id}/cancel`; `igloo task create|show|list|cancel`.

### Acceptance

- Scenario tests for every command and `plan` branch.
- Two tasks with the same tool and lockfiles build one agent snapshot.
- A task over its `max_turns` fails; a cancelled task stops its sandbox.

## P4.3 Harnesses and transcripts

- Contract changes: `igloo-api`, `schemas/openapi.json`

### Deliverables

- `Harness` trait: the job of a turn (argv, environment, secrets) from the goal or the review
  comments, and the parsing of the job's output into transcript entries.
- `ClaudeCodeHarness`: `claude -p` with streamed JSON output, resuming the task's session on
  later turns; the credential from `claude setup-token` as a secret. `CommandHarness`: any
  command, its raw output as one transcript entry per line.
- `TranscriptEntry`: message, tool call, tool result, command, error; each with its turn and
  position. Transcripts are derived from the turn jobs' logs on read; nothing new is stored.
- `GET /v1/tasks/{id}/transcript?after=<position>`: entries from a position on, the position to
  read next, and whether a turn runs. Clients poll it; `igloo task transcript <id> --follow`.

### Acceptance

- Recorded Claude Code output parses into the expected entries (fixture test).
- A secret's value in the tool's output appears masked in the transcript.

## P4.4 Revisions from tasks

- Contract changes: `igloo-api`, `schemas/openapi.json`, ports (`Forge`)

### Deliverables

- After each turn, a job in the task's sandbox commits what the tool left uncommitted and writes
  a git bundle of the commits since the task's commit to its output, base64 encoded; the server
  decodes it from the job's logs. Sandboxes hold no API credentials and need only `git` and
  `base64`.
- `Forge::import`: fetches a bundle into the mirror and pushes `igloo/tasks/<task id>`.
- The first non-empty turn opens the task's change from that branch; later ones add revisions
  (P3.4). A turn without commits ends the task as failed with its transcript.

### Acceptance

- A turn that commits produces a revision whose head is its last commit; checks run on it.
- The task's sandbox never holds the forge token.

## P4.5 Review

- Contract changes: `igloo-api`, `schemas/openapi.json`, `igloo-core` change events

### Deliverables

- Change comments: `POST /v1/changes/{id}/comments`, per revision, optionally on a file and
  line; `igloo change comment <id>`.
- `RequestChanges` (`POST /v1/changes/{id}/request-changes`, `igloo change request-changes`):
  carries the comments since the previous request; a reactor gives them to the task as its next
  turn's prompt. A request made while a turn runs waits for that turn's revision.
- Approvals from `Actor::Agent` are refused (`change.agent_approval`).
- A merged change finishes its task; a closed one cancels it.

### Acceptance

- Requesting changes on a task's change starts a turn whose prompt holds the comments.
- An agent's approval is refused; a human's merges as in phase 3.

## P4.6 MCP server

- Contract changes: MCP tool list (new surface), new dependency (MCP server crate)
- Read first: architecture §10

### Deliverables

- `transport/mcp/`: streamable HTTP at `/mcp`, the bearer token as for REST.
- Tools, thin adapters over the command bus and queries: `task.create`, `task.get`,
  `task.list`, `task.transcript`, `task.cancel`, `change.get`, `change.comment`,
  `change.request_changes`. No approve or merge tool.
- `docs/mcp.md`: adding Igloo to Claude Code, Codex and Pi.

### Acceptance

- An MCP client lists the tools and creates a task end to end against a test server.

## P4.7 Igloo on Igloo, with agents

- Contract changes: `.igloo/agents.toml`, outcome events (task reference)

### Deliverables

- Igloo's `.igloo/agents.toml`: `claude-code` as default tool, installed in the agent snapshot.
- Outcomes record the task, tool and turns of a change that has one.
- An exit-gate test with `CommandHarness` and a scripted agent covering items 1-7; an opt-in
  Linux test with Claude Code (`IGLOO_TEST_AGENT`, needs a token).
- `docs/igloo-on-igloo.md`: tasks from the editor.

### Acceptance

- The exit gate passes against a scripted agent; the opt-in test passes with Claude Code.

## Out of scope

- Web UI: review happens through MCP and the CLI.
- Agent tokens, grants and Cedar policy: agents do not call Igloo during a turn.
- Egress allowlists: agent sandboxes have the host's network.
- Budgets in money or tokens; earned autonomy and auto-merge.
- A second reviewing agent; harnesses other than Claude Code and `command` beyond a stub for
  Codex.

## Decisions

1. **Commits leave a sandbox as a git bundle** in a job's output; the server pushes
   `igloo/tasks/<task id>`. The forge token never enters an agent's sandbox.
2. **A task's sandbox keeps running between turns**, so a turn resumes the tool's session at
   once. An idle limit comes with budgets.
3. **Coding tools are installed into an agent snapshot**: the tool's `install` command over the
   warm snapshot, cached by key. CI sandboxes never carry agent tools.
4. **MCP neither approves nor merges.** Approval and merge stay deliberate human actions through
   the CLI.
