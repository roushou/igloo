# Phase 6: Workspaces

Exit gate, for the Igloo repository itself, with no clone of it on the owner's machine:

1. From the console or `igloo shell`, the owner opens a workspace on `main` and gets a shell in it
   within seconds, with dependencies already built.
2. In that shell the owner edits with Neovim, runs the checks, commits, and pushes a branch to
   Igloo. The push opens a change, whose checks run as in phase 3.
3. The owner stops the workspace and opens it again later: uncommitted work and shell history are
   still there.
4. While an agent's task runs, the owner opens a terminal in its sandbox, watches it work, and can
   take over.
5. The change merges in Igloo; `main` on the forge follows within a minute, and the change's
   branch is gone from both.

Order: P6.1 -> P6.2 and P6.3 in parallel -> P6.4 -> P6.5 -> P6.6.

Read first for every task: ADR 0013, ADR 0011, architecture §7 and §8.

## P6.1 Interactive terminals

- Contract changes: `proto/`, `igloo-worker-protocol`, `igloo-api`, `schemas/openapi.json`; new
  dependencies: a WebSocket implementation for axum (its `ws` feature) and `@xterm/xterm` with its
  fit addon in `web/`

### Deliverables

- The worker runs a process in a sandbox with a pseudo-terminal: input, output, resize and exit
  stream both ways over the existing gateway session, multiplexed with jobs.
- `GET /v1/sandboxes/{id}/terminal` upgrades to a WebSocket carrying the terminal; authenticated
  like REST. A terminal session ends when its process exits or the socket closes.
- A reusable terminal component in the console, themed with the code surface of the Console
  design.

### Acceptance

- A shell in a sandbox echoes input, honours resizes and reports its exit code (worker test over
  the OCI runtime, gateway test over the memory adapters).
- A sandbox's terminal is refused to callers without the token.

## P6.2 Workspaces

- Depends on: P6.1
- Contract changes: `igloo-core` (new entity), `igloo-api`, `schemas/openapi.json`, migrations if
  any

### Deliverables

- `Workspace` resource (platform): owner, repository, branch, the sandbox it runs in, the layer
  its last stop sealed, phase `Starting | Running | Stopping | Stopped`.
- Opening forks the repository's warm snapshot at the branch head (P3.3), plus the workspace's
  sealed layer when it has one; stopping seals the sandbox's changes, then stops it. Network is
  allowed; the repository's secrets are granted as for agents.
- An idle workspace (no terminal attached for 2 hours) stops itself.
- `POST /v1/repos/{id}/workspaces`, `GET /v1/workspaces`, `POST /v1/workspaces/{id}/start|stop`,
  `DELETE /v1/workspaces/{id}`; `igloo workspace create|list|start|stop|delete`.

### Acceptance

- A file written in a workspace survives a stop and a start (scenario tests and a worker test).
- Two workspaces of the same repository and branch share the warm snapshot's layers.

## P6.3 Git hosted by Igloo

- Contract changes: `igloo-api`, `schemas/openapi.json`, ports (`Forge` gains mirroring)

### Deliverables

- Igloo serves each repository's copy over git's smart HTTP protocol at
  `/git/<repository id>.git`, authenticated with the bearer token. Clones, fetches and pushes work
  with stock git.
- Pushes: a branch push updates Igloo's copy; a push to the default branch is refused (it moves
  only by merge); a pushed branch with no change opens one, a branch with an open change records
  a revision.
- Mirroring: after every merge, and on a schedule, Igloo pushes the default branch to the forge;
  change branches are mirrored while their change is open and deleted when it ends.
- Workspaces and agent sandboxes use Igloo as their `origin`, through a credential helper that
  needs no forge token.

### Acceptance

- `git clone`, `git push` of a branch and `git fetch` work against Igloo with the token and fail
  without it (integration test).
- A push to the default branch is refused with a message naming the merge.
- A merge reaches the forge's default branch.

## P6.4 Shell from anywhere

- Depends on: P6.1, P6.2

### Deliverables

- `igloo shell [<workspace>]`: a terminal in a workspace from the owner's terminal, over the
  WebSocket of P6.1, starting the workspace when stopped. With no argument, the owner's most
  recent workspace of the current repository.
- The owner's dotfiles: a repository setting naming a dotfiles repository and an install command,
  run once when a workspace is created.
- `docs/workspaces.md`: opening a workspace, Neovim, coding tools, and the escape hatch when Igloo
  itself is down.

### Acceptance

- `igloo shell` attaches, resizes with the local terminal and exits with the remote shell.

## P6.5 Attach to agents

- Depends on: P6.1

### Deliverables

- A task's sandbox terminal from its page: read-only by default; "Take over" makes it writable
  and pauses the task after its current turn.
- Handing a taken-over task back resumes it with a turn whose prompt names what the person
  changed.

### Acceptance

- A read-only attach cannot write to the sandbox; a take-over can, and the task waits for it to
  be handed back.

## P6.6 Workspaces in the console

- Depends on: P6.2, P6.5

### Deliverables

- A Workspaces page: the owner's workspaces with their phase, branch and last activity; open,
  stop and delete; create from a branch.
- A workspace page: the terminal full height, with a side panel for the branch's change, its
  checks, and recent pushes.
- Now shows running workspaces beside running tasks.

### Acceptance

- Component tests for the workspace states; the exit gate holds on the host, checked by the
  owner.

## Out of scope

- A browser code editor; port previews of servers running in a workspace.
- More than one person per workspace; sharing a terminal.
- Moving the forge mirror to another forge.
