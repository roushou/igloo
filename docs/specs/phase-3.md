# Phase 3: Changes

Exit gate, for the Igloo repository itself:

1. `igloo change create --branch <name>` makes a change whose first revision is that branch's
   head on the forge, over `main`.
2. Igloo runs the checks of `.igloo/pipeline.toml` on every revision, in a sandbox forked from a
   warm snapshot of the repository; results and logs are visible with `igloo change show <id>`.
3. `igloo change merge <id>` merges a revision whose checks passed: Igloo pushes `main`. A change
   touching a protected path of `.igloo/agents.toml` needs a human approval first.
4. Every merge records an outcome event: change, revision, checks, approvals, merged commit.
5. Secrets reach checks as environment variables and never appear in events, logs or responses.
6. GitHub Actions still builds every commit independently.

Order: P3.1 and P3.2 in parallel -> P3.3 -> P3.4 -> P3.5 -> P3.6 -> P3.7.

## P3.1 Secrets

- Contract changes: migrations, `igloo-api`, `schemas/openapi.json`, ports

### Deliverables

- `SecretStore` port: per-repository named secrets, encrypted at rest with a key from
  configuration (`IGLOO_SECRETS_KEY`), memory and Postgres adapters with a conformance suite.
- `PUT /v1/repos/{id}/secrets/{name}` (write-only), `GET /v1/repos/{id}/secrets` (names only),
  `DELETE`. `igloo secret set <repo> <name>` reads the value from stdin.
- Jobs name secrets; the gateway resolves them into the lease grant's environment. Job specs,
  events and logs hold names only.

### Acceptance

- A job naming a secret sees its value; no event, log entry or response contains it.
- A secret of one repository is not readable through another.

## P3.2 Repo and forge

- Contract changes: `igloo-api`, `schemas/openapi.json`, ports, migrations (none expected)

### Deliverables

- `Repo` entity: forge (`github`), owner/name, default branch, the secret holding the forge
  token. Unique per forge and full name.
- `Forge` port: resolve a ref to a commit, fetch commits into the server's mirror, push a
  branch, compare-and-swap push of the default branch. Adapters: `GitHubForge` (HTTPS with the
  token) and `LocalForge` (a bare repository on disk) for tests and development.
- A bare mirror per repository under the data directory, driven by the `git` binary.
- `POST /v1/repos`, `GET /v1/repos/{id}`; `igloo repo add github.com/<owner>/<name>`.

### Acceptance

- Both adapters pass the forge conformance suite (against local bare repositories).
- Pushing the default branch fails when it moved since the commit the push expects.

## P3.3 Repository snapshots

- Contract changes: `igloo-api`, `schemas/openapi.json`, snapshot manifest (environment)

### Deliverables

- Image import keeps the image's `Env` in the snapshot manifest (format v3); sandboxes start
  with it under their own environment. Jobs always run in `/workspace`, so `WorkingDir` is not
  kept.
- `RepoSnapshots`: a snapshot of a repository at a commit is a base snapshot plus a layer holding
  the checkout under `workspace/`, with a shallow `.git` at that commit.
  `POST /v1/repos/{id}/snapshots`, `igloo repo snapshot`.
- Warm snapshot reuse: a warm snapshot is keyed by its base, its build command and the lockfiles
  at its commit, and recorded on the repository. A checkout over a warm snapshot deletes the files
  removed since the warm commit and replaces its `.git`. Building warm snapshots is part of runs
  (P3.5).

### Acceptance

- A sandbox from a repository snapshot runs `git log -1` and prints the commit.
- Two commits with the same lockfile have the same warm key; a checkout over the warm snapshot
  drops the files the second commit deleted.

## P3.4 Changes and revisions

- Contract changes: `igloo-api`, `schemas/openapi.json`, migrations (none expected)

### Deliverables

- `Change` entity: repository, target branch, title, revisions (head commit, base commit),
  phase `open | merged | closed`. Revisions only append; merged at most once.
- `POST /v1/repos/{id}/changes { branch, title }`: the server fetches the branch from the forge
  into its mirror and records its head as the first revision. `POST .../changes/{id}/revisions`
  records the branch's new head. Developers push branches with their own git credentials.
- `igloo change create --branch <name> [--title]`, `igloo change push <id>`,
  `igloo change show <id>`, `igloo change checkout <id>`.

### Acceptance

- Scenario tests for the change lifecycle; an end-to-end test against `LocalForge` creates a
  change and a second revision.

## P3.5 Pipelines and runs

- Contract changes: `schemas/` (pipeline JSON Schema), `igloo-api`, `schemas/openapi.json`

### Deliverables

- `.igloo/pipeline.toml`: base image, `warm` command, environment, secrets, and named checks
  (command, timeout). JSON Schema in `schemas/pipeline.schema.json`.
- `Run` entity in the `ci/` product module: one per revision, pinned to its commit and warm
  snapshot; each check runs as a job in one sandbox forked from it; phase per check and
  overall, terminal final.
- A reactor starts a run for every new revision; `igloo change show` lists runs and checks;
  `igloo logs` follows a check.
- A run without a warm snapshot for its key builds one first: the pipeline's `warm` command in a
  sandbox of the cold checkout, sealed and recorded on the repository.
- The pipeline's `image` is imported once per repository and recorded on it; `base` names a
  snapshot instead, and neither checks out over nothing, for runtimes that use the host's tools.
- A run's checks are submitted together as jobs of one sandbox.
- `.igloo/pipeline.toml` for Igloo: format, clippy, tests.

### Acceptance

- A revision with a failing check ends its run `failed`; the next revision gets a new run.
- A run's sandbox is stopped once the run ends.

## P3.6 Review and merge

- Contract changes: `igloo-api`, `schemas/openapi.json`

### Deliverables

- Approvals on a revision by humans; `igloo change approve <id>`.
- `igloo change merge <id>`: allowed when the latest revision's run passed and, if it touches a
  protected path from `.igloo/agents.toml` at the target branch, a human approved it. Igloo
  pushes the target branch with compare-and-swap; if the branch moved, the merge is refused
  and a rebased revision is needed.
- Protected paths are read from the target branch, never from the change.

### Acceptance

- Merge is refused with failing checks, with a missing approval on a protected path, and when
  the target branch moved; it succeeds otherwise, once.

## P3.7 Outcomes and the exit gate

- Contract changes: event schemas (outcome events)

### Deliverables

- An `Outcome` per ended change (`igloo.outcome.recorded`): verdict, the judged revision, its
  run's check results, its approvals, the merged commit and the commits merged. A commit on the
  default branch whose message reverts one of them (`This reverts commit <sha>`) is recorded
  against it (`igloo.outcome.reverted`), found when a later change event fetches the branch.
- End-to-end test of the exit gate against a repository on disk; `docs/igloo-on-igloo.md` for the
  same flow against GitHub.

### Acceptance

- The exit gate, items 1 to 5, as tests; item 6 by inspection.

## Decisions

1. **Forge credentials: a fine-grained personal access token** stored as a repository secret. The
   GitHub App comes with external repositories (phase 8).
2. **Changes start from branches on the forge**, pushed by the developer, who works on a machine
   with git access to the forge. Agents in sandboxes push through Igloo in phase 4.
3. **Merges are fast-forward only**. A target branch that moved needs a rebased revision, which
   reruns the checks.
4. **Server-side git is the `git` binary**; the server needs `git` installed.
5. **Secrets use XChaCha20-Poly1305** (`chacha20poly1305`) with random 24-byte nonces and the
   repository and name as associated data.
