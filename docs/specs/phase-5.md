# Phase 5: Console

Exit gate, for the Igloo repository itself, through the tunnel at `http://127.0.0.1:7000`:

1. `igloo-control` on the host serves the console at `/`; `deploy/vps.sh update` builds and installs
   it.
2. The Now page shows what waits on the user, what runs, and recent activity, and updates without a
   reload. It shows when the connection to the server is lost and when it returns.
3. From the console alone, the user creates a task, follows its transcript, reads its change's diff
   and check logs, comments on a line, requests changes, approves and merges.
4. A run that errored shows its reason where it is listed. A failed check opens at its log's first
   error.
5. The System page shows each worker's connection, allocated CPU and memory, disk and layer cache,
   the running sandboxes and the repository's snapshots.
6. Secrets are set and deleted from the console; values are never shown or returned.

Order: P5.1 -> P5.2, P5.3, P5.4 and P5.5 in parallel -> P5.6 -> P5.7 and P5.8 in parallel -> P5.9.

Read first for every task: ADR 0012, architecture §10.

## P5.1 Console foundation and hosting

- Contract changes: configuration (`IGLOO_WEB_DIR`); new dependencies: `tower-http` (`fs`) and the
  `web/` stack of ADR 0012
- Read first: ADR 0012, `docs/configuration.md`, `deploy/vps.sh`

### Goal

An empty console, signed in with the token, served by the server, built in CI and on deploy.

### Deliverables

- `web/`: TanStack Start in SPA mode, Bun, Tailwind CSS, shadcn/ui, Biome, Vitest. API types and a
  typed client generated from `schemas/openapi.json` with one script; the generated file is
  committed and CI fails when it is stale.
- The shell: a left rail with the repository switcher (`GET /v1/repos`), the pages of P5.6 to P5.8
  as placeholders, light and dark themes following the system, and a command palette (⌘K) that opens
  a resource by pasted id (`repo_`, `chg_`, `task_`, `run_`, `job_`, `sbx_`).
- Sign-in: a page that asks for the token, checks it with `GET /v1/repos`, and keeps it in
  `localStorage`. A 401 anywhere returns to it.
- `igloo-control` serves `IGLOO_WEB_DIR` at `/` when set: files by path, `index.html` for any other
  `GET` outside `/v1` and `/mcp`. Unset, `/` answers 404. Hashed assets are cached
  immutably; `index.html` is not cached.
- `deploy/vps.sh`: installs Bun, builds `web/`, installs the output to `/usr/local/share/igloo/web`,
  and adds `IGLOO_WEB_DIR` to an existing `control.env` that lacks it.
- CI: a `web` job running `bun install --frozen-lockfile`, `biome ci`, the type check, the tests,
  the build and the generated-types check. Igloo's pipeline installs Bun in its warm command and
  gains a `web` check.
- `AGENTS.md` (workspace, commands) and `docs/conventions.md` (a short web section: file layout,
  data fetching through TanStack Query, no direct `fetch` outside the API client) updated.

### Acceptance

- Given `IGLOO_WEB_DIR`, `GET /` and `GET /tasks/x` return `index.html`, `GET /assets/<file>`
  returns the file, and `GET /v1/repos` is still the API.
- A wrong token stays on the sign-in page with an error; a right one opens the shell.
- `docs/configuration.md` lists `IGLOO_WEB_DIR`.

### Out of scope

- Any page content. Server-side rendering. Authentication other than the bearer token.

## P5.2 Event stream

- Contract changes: `igloo-api`, `schemas/openapi.json`
- Read first: ADR 0004, `ports/event_log.rs`

### Deliverables

- `GET /v1/events`: server-sent events over the event log, one per committed event: `id` is the
  sequence, `event` is the kind (`igloo.run.passed`), `data` holds the sequence, kind, resource type
  and id, repository id when the resource has one, and time. Domain payloads are not included.
- `?repo=<id>` filters by repository; `?after=<sequence>` or `Last-Event-ID` resumes. Keep-alive
  comments every 15 seconds.
- `web/`: one stream per tab, read with `fetch` so the token is sent as a header; reconnects with
  backoff from the last id; maps each event to the TanStack Query keys of its resource; exposes the
  connection state (live, reconnecting, offline) to the shell.

### Acceptance

- A client that reconnects with `Last-Event-ID` receives every event after it, in order, once.
- `?repo=` excludes events of other repositories.
- A command's event reaches a connected client without polling (test over the memory adapters).

## P5.3 Read models for screens

- Contract changes: `igloo-api`, `schemas/openapi.json`

### Deliverables

- List filters: `?phase=` (repeatable) and `?order=newest` on a repository's tasks and changes.
- `GET /v1/repos/{id}/runs`: the repository's runs, newest first, with cursor pagination.
- Timing: `started_at` and `ended_at` on jobs, runs and checks, derived from their events.
- Merge readiness on `ChangeResource`: `checks` (passed, failed, running, missing), `approval`
  (`not_required`, `required`, `given`, with the protected paths that require it) and
  `fast_forward` (whether the latest revision's base is the target's head in the mirror). Computed
  by the same rules the `Merger` applies; merge stays authoritative.
- `GET /v1/workers`: id, labels, connection, schedulability, capabilities, CPU and memory allocated
  to its sandboxes, last heartbeat.

### Acceptance

- A change touching a protected path without approval reports `approval: required` with that path,
  and merging it is refused; after approval both agree.
- `?phase=open` returns only open changes; `?order=newest` reverses the order.

## P5.4 Worker usage

- Contract changes: `proto/`, `igloo-worker-protocol`, `igloo-api`, `schemas/openapi.json`

### Deliverables

- Workers report usage every 30 seconds and on registration: data directory disk total and free,
  layer cache size and limit, sandboxes held.
- `GET /v1/workers` includes the latest usage and its time.

### Acceptance

- A worker's report appears on `GET /v1/workers` within one report interval.
- A worker one protocol version behind, without usage, is still accepted.

## P5.5 Diffs

- Contract changes: ports (`Forge`), `igloo-api`, `schemas/openapi.json`

### Deliverables

- `Forge::diff(repo, base, head)`: the files changed, each with path, previous path, status, lines
  added and removed, binary flag, and unified patch.
- `GET /v1/changes/{id}/diff?revision=<n>` (latest by default) from the repository's mirror. A file
  whose patch exceeds 256 KiB is returned without it and marked truncated.

### Acceptance

- A revision that adds, modifies, renames and deletes files returns each with the right status and
  counts (`GitForge` and the memory forge pass a shared conformance test).

## P5.6 Now, tasks and logs

- Depends on: P5.1, P5.2, P5.3

### Deliverables

- One status module mapping every phase enum to six states, used by every page:

  | State     | Phases                                                                                       |
  | --------- | -------------------------------------------------------------------------------------------- |
  | Needs you | task `awaiting_review`; open change whose checks passed or that requires approval            |
  | Running   | task `preparing`, `working`; run `preparing`, `warming`, `checking`; job `leased`, `running` |
  | Passed    | run `passed`; job exited 0; change `merged`; task `done`                                     |
  | Failed    | run `failed`; job exited non-zero; task `failed`                                             |
  | Errored   | run `errored`; job `failed` with a reason                                                    |
  | Closed    | change `closed`; task and job `cancelled`; sandbox `stopped`                                 |

- Now (`/`): Needs you, Running (step, elapsed time, the previous duration of the same step, time
  since the last output) and Recent activity from the event stream.
- Tasks (`/tasks`, `/tasks/:id`): list by state; the transcript streaming, tool calls collapsed to
  one line and expandable, edits as diffs; the task's change; Create task and Cancel.
- Job log (`/jobs/:id`): the full log, following while it runs, search, jump to the first line
  matching `error`.

### Acceptance

- Component tests over recorded API responses for each state of each page.
- A task's transcript grows without reload as entries arrive.

## P5.7 Review

- Depends on: P5.5, P5.6

### Deliverables

- Changes (`/changes`, `/changes/:id`): the merge checklist from P5.3 above tabs for Diff,
  Checks, Comments and Timeline. Diff renders files with line comments; Checks lists checks with the
  selected one's log, the first failed one selected. Revision selector.
- Actions: comment (on the change or a line), request changes, approve, merge, close, record a new
  revision. A disabled action names its unmet rule. Each action's menu shows the equivalent CLI
  command.

### Acceptance

- Merge is disabled with its reason while readiness reports an unmet rule.
- A line comment is sent with its revision, path and line.

## P5.8 System and repository

- Depends on: P5.4, P5.6

### Deliverables

- System (`/system`): workers with allocation, disk and layer cache meters; sandboxes with the task
  or run that owns them; the repository's warm and agent snapshots with their keys.
- Repository (`/settings`): secret names with set (masked input) and delete; the forge location and
  token secret name.

### Acceptance

- A secret set from the page is listed by name; its value is never rendered after submission.

## P5.9 Exit gate

- Depends on: P5.7, P5.8

### Deliverables

- A Playwright test against a server with an embedded worker and a `command` tool: sign in, create a
  task, see it run and open a change, comment, request changes, approve and merge, all through the
  console, with the Now page updating live.
- `docs/console.md`: opening the console through the tunnel, and what each page shows.

### Acceptance

- The test passes in CI.
- The exit gate holds on the host for the Igloo repository.

## Out of scope

- Authentication beyond the bearer token, Tailscale, TLS.
- History, outcomes and metrics pages.
- Editing `.igloo/` files from the console.
- Mobile layouts beyond a usable Now page.
