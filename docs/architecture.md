# Igloo architecture

What exists and where it lives. How code is written: `conventions.md`. Why: `adr/`. Settings and
defaults: `configuration.md`.

## 1. Purpose

Igloo is a self-hosted platform where humans and agents build and maintain software. It forks
a warm environment instantly, runs untrusted code safely, reacts to events and records
outcomes. CI is the first product; the agent runtime is the second.

The loop Igloo exists to run, first for its own repository, then for any repository:

```
task -> an agent (any coding tool) works in a sandbox forked from a warm snapshot
     -> checks run -> change -> review in Igloo -> merge -> outcome recorded
```

In detail, for a change opened by a human or by a task's agent:

```
you / editor agent
      │ task.create (MCP)                      git push + igloo change create
      ▼                                                     │
 ┌─────────┐  fork warm     ┌─────────────────────┐         │
 │  Task   │──snapshot────► │ sandbox: harness    │         │
 └─────────┘                │ runs the coding     │         │
      ▲                     │ tool headless;      │         │
      │                     │ edits, commits      │         │
      │ request-changes     └─────────┬───────────┘         │
      │ (comments → new turn)         │ commits             │
      │                               ▼                     ▼
      │                       ┌───────────────────────────────────┐
      └───────────────────────│ Change   rev1 → rev2 → rev3 ...   │
                              │ (revisions only append)           │
                              └───────────────┬───────────────────┘
                                              │ each revision
                                              ▼
                              ┌───────────────────────────────────┐
                              │ Run: .igloo/pipeline.toml checks  │
                              │ in a fresh sandbox forked from    │
                              │ the warm snapshot (rebuilt when   │
                              │ a lockfile changes)               │
                              └───────────────┬───────────────────┘
                                              ▼
                   merge gate: checks passed on latest revision
                             ∧ protected paths approved by a human
                             ∧ main has not moved
                                              │ igloo change merge
                                              ▼
                       Igloo pushes main ──► Outcome recorded
                                              │ later `git revert` on main
                                              ▼
                                       outcome reverted
```

Principles:

1. Dependencies point inward: inbound adapters -> application -> domain. The domain does no I/O.
2. Decisions are pure and return events as data. The shell persists them and acts on them.
3. Infrastructure converges: controllers compare desired spec with observed status and act on
   the difference, repeatedly and idempotently.
4. The repository is the trust boundary. Grants, budgets, protected paths and autonomy attach
   to a `Repo`, never to global configuration.
5. Primitives are product-agnostic. Products (CI, agents) compose them; primitives never know
   products.
6. Build now what is expensive to retrofit; defer what is cheap to add behind an existing seam.
7. Igloo is repository number one, configured like any other. No Igloo-specific logic in the
   platform or products.

## 2. System

|           | `igloo-control`                                              | `igloo-worker`                                                |
| --------- | ------------------------------------------------------------ | ------------------------------------------------------------- |
| Role      | Decides what should happen, records what did                 | Runs untrusted code in sandboxes, reports back                |
| Instances | One (more later, for availability)                           | One per machine, many sandboxes each                          |
| Surfaces  | REST, SSE, MCP, worker gateway (gRPC server), forge webhooks | Worker protocol (gRPC client, outbound only), local admin API |
| Holds     | Database credentials, identity provider and forge App keys   | Its worker credential and job leases                          |
| Platform  | Anywhere                                                     | Linux (macOS for development, unisolated)                     |

In development, `IGLOO_EMBEDDED_WORKER=true` runs a worker inside the server process, so one
command starts a working Igloo. In production the worker dials one long-lived bidirectional
gRPC stream to the server. It authenticates
with a join token in development and mutual TLS in production. Snapshot and artifact bytes
move through `BlobStore` URLs, never over the stream. The two run as separate processes
under separate users, even on one machine.

```
  Developer machine                                     Forge (GitHub)
┌───────────────────────┐                        ┌──────────────────┐
│ igloo CLI / Rust SDK  │                        │ git host only:   │
│ editor agent          │──── MCP (/mcp) ──┐     │ fetch, push      │
│ (Claude Code, Codex)  │                  │     │ revision branches│
└──────────┬────────────┘                  │     │ and merges       │
           │ REST /v1 + SSE                │     └────────▲─────────┘
           │ bearer token                  │              │ repository token
           ▼                               ▼              │
┌─────────────────────────────────────────────────────────┴──────────┐
│                         igloo-control                              │
│   REST · SSE · MCP (:7000)          worker gateway, gRPC (:7001)   │
└───────┬──────────────────────────────────────────▲─────────────────┘
        │ SQL                                      │ one bidirectional gRPC stream,
        ▼                                          │ dialed by the worker:
┌───────────────┐                                  │ assignments, leases ↓
│   Postgres    │  event log, projections,         │ heartbeats, logs, results ↑
│               │  checkpoints                     │
└───────────────┘                                  │
                                   ┌───────────────┴───────────────────┐
    layers and snapshots           │     igloo-worker (one per machine)│
  ◄──── presigned blob URLs ──────►│  Linux, root, OCI runtime, overlay│
                                   │  ┌─────────┐ ┌─────────┐ ┌──────┐ │
                                   │  │ sandbox │ │ sandbox │ │ ...  │ │
                                   │  │ (check) │ │ (task)  │ │      │ │
                                   │  └─────────┘ └─────────┘ └──────┘ │
                                   └───────────────────────────────────┘
```

## 3. Core loop

Every state change goes through one path:

```
callers       REST · MCP · gateway · reactors
                     │  Command
                     ▼
┌─────────────── CommandBus ─────────────────┐
│ tower layers: tracing → timeout → authz    │  the only write path
└────────────────────┬───────────────────────┘
                     ▼
              CommandHandler   usually the generic EntityHandler
                     │ load the entity (replay its events)
                     ▼
              entity method    pure (igloo-core): rejects with a typed
                     │         error, or records events
                     ▼
           EntityStore::commit ── one transaction, one entity ──┐
                     │                                          │
                     ▼                                          ▼
           events: global log, gapless sequence;          projections
           also the outbox                                (query tables)
                     │ LISTEN/NOTIFY, polling as fallback
         ┌───────────┴─────────────┐
         ▼                         ▼
  Controller<R>               Reactor
  level-triggered:            event → commands;
  spec vs status →            per-reactor checkpoint,
  pure plan → actions         at least once, idempotent
         │                         │
         └──────► CommandBus ◄─────┘   cross-entity effects go
                                       through events
```

## 4. Crates

| Crate                   | Kind      | Role                                                   | Depends on (internal)                   |
| ----------------------- | --------- | ------------------------------------------------------ | --------------------------------------- |
| `igloo-core`            | lib       | Pure domain: vocabulary and platform primitives        | -                                       |
| `igloo-api`             | lib       | Public API contract: REST types, problems, OpenAPI     | core                                    |
| `igloo-worker-protocol` | lib       | Server <-> worker gRPC contract                        | core                                    |
| `igloo-control`         | lib + bin | Server: bus, controllers, ports, adapters, inbound, CI | core, api, worker-protocol, worker, git |
| `igloo-worker`          | lib + bin | Worker: runtimes, snapshots, executors, reconciler     | core, worker-protocol                   |
| `igloo-rs`              | lib       | Rust SDK, library name `igloo`                         | api                                     |
| `igloo-cli`             | bin       | CLI, binary `igloo`                                    | rs, core, git                           |
| `igloo-git`             | lib       | Git client over the `git` binary                       | core                                    |

```
                    igloo-core  (pure, no I/O)
                  ▲      ▲       ▲
       ┌──────────┘      │       └──────────────┐
   igloo-api   igloo-worker-protocol            │
    ▲    ▲            ▲        ▲                │
    │    │            │        └── igloo-worker ┘
    │    │            │                 ▲
    │    └── igloo-control ─────────────┘  embeds a worker in development;
    │                                      the worker never imports control
igloo-rs (SDK)
    ▲
igloo-cli (`igloo`)

igloo-git (git client) depends on igloo-core; igloo-control and igloo-cli use it.
```

Rules:

- A crate earns its existence: a deployable binary, a published library, a contract shared by
  separate programs, or isolation of heavy or platform-specific dependencies. Everything else
  is a module.
- A crate owns its concern. It declares what it needs as traits it owns and exposes types it
  owns; it never reaches into another crate's internals.
- `igloo-worker` never depends on `igloo-control`; it reaches the server only through the
  protocol. `igloo-control` composes everything and may embed a worker.
- Contract crates hold data and conversions only.
- `igloo-core`, `igloo-api` and `igloo-rs` are published. `igloo-core` and `igloo-api` carry no
  stability promise of their own.
- A dependency used by one crate is declared in that crate. It moves to
  `[workspace.dependencies]` when a second crate uses it.

## 5. Layout

```
igloo/
├── AGENTS.md  Cargo.toml  rust-toolchain.toml  rustfmt.toml  clippy.toml  deny.toml
├── docs/                     architecture, conventions, adr/, specs/, prompts/
├── proto/igloo/worker/v1/    worker protocol
├── schemas/                  generated and committed: openapi.json, pipeline.schema.json
├── dev/compose.yaml          local Postgres
├── crates/
│   ├── igloo-core/src/
│   │   ├── lib.rs            re-exports the vocabulary: Id, Digest, Timestamp, Actor, Labels,
│   │   │                     Entity, Resource, Event, Plan (one module per concept)
│   │   ├── testing/          Scenario harness (feature "testing")
│   │   └── sandbox/ snapshot/ job/ worker/ repo/
│   ├── igloo-control/src/
│   │   ├── main.rs           reads configuration, serves, shuts down gracefully
│   │   ├── config.rs         `IGLOO_*` configuration
│   │   ├── composition.rs    `Server`: the only place naming concrete adapters
│   │   ├── app/              Command, CommandBus, handlers, Controller, Reactor, Extension
│   │   ├── platform/         one module per primitive: commands, reconcilers
│   │   ├── ci/               pipeline spec, runs, forge integration
│   │   ├── agents/           tool settings, tasks, harnesses, transcripts
│   │   ├── ports/            one trait per side effect, with its conformance suite
│   │   ├── adapters/         memory/, postgres/, ...
│   │   └── inbound/          rest/, mcp, gateway/
│   ├── igloo-worker/src/     runtime/, snapshot/, executor/, reconciler, gateway client
│   │   (tests/)              end-to-end tests: real server, embedded worker, Postgres
│   ├── igloo-git/src/        Git, Repository, typed refs, URLs and paths; testing::Fixture
│   │                         (feature "testing")
│   ├── igloo-rs/src/         Client, log streaming, `Layer::from_dir`, `Client::run`
│   └── igloo-cli/src/        main.rs: a thin binary over the SDK and igloo-git
└── .github/                  CI workflow, CODEOWNERS (protected paths)
```

## 6. Domain model

**Entities** are built with `new` and changed with methods. A method rejects a change or
records events, which are applied to the state at once and committed with it. **Resources** are entities with a desired `spec`, an observed `status`, a
`generation` and a pure `plan` that says what to do about the gap.

Vocabulary (re-exported at the root of `igloo-core`):

| Type        | Definition                                                                      |
| ----------- | ------------------------------------------------------------------------------- |
| `Id<T>`     | TypeID: `<prefix>_<26 base32 chars>` over a UUIDv7. Sortable, typed per entity. |
| `Digest`    | blake3 content address, `<algorithm>:<hex>`.                                    |
| `Timestamp` | UTC instant, RFC 3339 on the wire.                                              |
| `Actor`     | `Human { user }`, `Agent { agent, principal }`, `System { component }`.         |
| `Labels`    | Validated key/value metadata on resources; list endpoints filter by equality.   |
| `Event`     | A fact with a stable `kind` and `SCHEMA_VERSION`, stored in the event log.      |

Primitives:

| Primitive | Kind                      | Invariants                                                                 | Phase |
| --------- | ------------------------- | -------------------------------------------------------------------------- | ----- |
| Sandbox   | resource                  | Sealed source snapshot; deny-all network by default; terminal phases final | 1     |
| Snapshot  | value (content-addressed) | ID is the digest of its manifest; immutable once sealed; forkable          | 1     |
| Seal      | entity                    | Ends once: sealed with a snapshot or failed; completed by the layer upload | 2     |
| Build     | resource                  | A command over a snapshot, sealed and recorded on a repo under a key; once | 4     |
| Worker    | resource                  | Capabilities match its protocol version                                    | 1     |
| Job       | resource                  | Leased once; an expired lease fails it with `lease_lost`; tokens increase  | 1     |
| Repo      | entity                    | Unique per forge and full name; owns trust settings                        | 3     |
| Change    | entity                    | A diff from a base commit; revisions only append; merged at most once      | 3     |
| Run       | resource (CI)             | Checks of one revision; pinned to a commit and a snapshot; terminal final  | 3     |
| Outcome   | entity (CI)               | One per ended change; records once, then at most one revert                | 3     |
| Task      | resource (agents)         | One agent on one goal in one sandbox, in turns; ends once, done or not     | 4     |
| Grant     | value                     | Names an agent, its principal, one repo, a scope and an expiry             | 4     |
| Workflow  | entity                    | Decisions are a pure function of history; effects through idempotent jobs  | 5     |

Job kinds are a closed enum (`Execute`, `BuildSnapshot`); product variety lives in the steps
inside `Execute`. Workers advertise `Capabilities`; jobs derive `Requirements`;
`Capabilities::satisfies` (pure) explains every mismatch. Placement uses it, and the worker
re-checks it on every lease.

**Repo trust settings**: protected paths, agent grants, budgets and the autonomy stage per
task category. They come from `.igloo/agents.toml` in the repository and from recorded
outcomes. There is no global equivalent.

## 7. Control plane

```
composition.rs  Server: the only place naming concrete adapters
┌────────────────────────────────────────────────────────────────────┐
│ inbound/  rest/ (REST, SSE)  mcp (MCP)  gateway/ (worker gRPC)     │
├────────────────────────────────────────────────────────────────────┤
│ app/        CommandBus · Controller · Reactor · TaskSupervisor     │
│             PlatformBuilder ◄── each module registers an Extension │
├──────────────────────────────┬─────────────────────────────────────┤
│ platform/  (primitives)      │ products: import platform,          │
│  sandbox snapshot seal build │           never the reverse         │
│  job worker repo change      │  ci/      pipeline, runs, merge,    │
│  checkout trust              │           outcomes                  │
│                              │  agents/  tasks, harnesses,         │
│                              │           transcripts, settings     │
├──────────────────────────────┴─────────────────────────────────────┤
│ ports/  one trait per side effect, each with a conformance suite   │
├────────────────────────────────────────────────────────────────────┤
│ adapters/  memory/  postgres/  fs_blob  git_forge  oci_registry    │
└────────────────────────────────────────────────────────────────────┘
```

- **CommandBus.** A typed registry; each dispatch goes through tower layers. It is the only
  write path, so authorization and audit apply to every inbound adapter identically.
- **Idempotency.** The REST API replays the response of a keyed request through the
  `IdempotencyStore` port, scoped to the actor: same key and request replay it for 24 hours,
  another request under the key is refused, and a failed request keeps no record.
- **Handlers.** The generic `EntityHandler` loads the target, calls one entity method and
  commits the state with its recorded events. Most commands need no handler code.
- **Controller<R>.** Level-triggered driver per resource kind: a deduplicating work queue fed
  by `R`'s events and a periodic resync, `Plan::Recheck` requeues, backoff on error, bounded
  concurrency. Runs a `Reconciler<R>` that executes the plan's actions idempotently.
- **Reactor.** Turns events into commands. A runner feeds it the log in order and saves a
  per-reactor checkpoint after each event, so restarts resume without reprocessing.
- **Extension.** Each module (sandboxes, jobs, CI, later agents) registers its commands,
  controllers, reactors and routes into the `PlatformBuilder` through one `Extension` impl.
  Platform modules never import product modules. Compile time, in process.
- **Ports.** `Clock`, `IdGenerator`, `EntityStore<E>`, `EventLog`, `Checkpoints`, `JobQueue`, `BlobStore`,
  `PolicyEngine`, `TaskSupervisor`. Each has a memory adapter and a conformance suite.

## 8. Worker

```
  gRPC stream to igloo-control (outbound only)
        │ Assignment: desired sandboxes, derived, pushed again on change
        │ lease grants: job, secrets environment, fencing token
        ▼
┌──────────────┐  converge  ┌───────────────────────────────────────┐
│  Reconciler  │──────────► │ SandboxRuntime                        │
└──────────────┘            │  OciRuntime (youki, crun, runc)       │
                            │  ProcessRuntime (development only)    │
┌──────────────┐            └──────────────┬────────────────────────┘
│  Executor    │ Execute,                  │ root file system
│  per job kind│ BuildSnapshot             ▼
└──────┬───────┘                ┌──────────────────────────────────┐
       │                        │ Rootfs: overlay mount            │
       │                        │   upper: private to the sandbox  │
       │                        │   lower: cached layers ◄─────────┼── LayerCache:
       │                        │ cgroup v2 limits; own network    │   verified by
       │                        │ namespace, loopback only unless  │   digest, LRU,
       │                        │ the sandbox allows network       │   fetched through
       │                        └──────────────────────────────────┘   presigned URLs
       ▼
 logs and results ──► local outbox ──► stream, until acknowledged, with the lease token
 Seal: upper directory → new layer → upload → content-addressed Snapshot
```

- **SandboxRuntime.** Creates, starts, executes in, stops and destroys sandboxes.
  Implementations: `ProcessRuntime` (no isolation; development and tests only, refused
  outside development mode), `OciRuntime` (containers through `youki`, `crun` or `runc`), `FirecrackerRuntime` (microVMs).
  `OciRuntime` runs each job as its own container over the sandbox's root file system: fresh
  namespaces, the sandbox's cgroup v2 limits without swap, and the host's network when the
  sandbox allows network access. Otherwise the worker creates one network namespace per
  sandbox over netlink, with only loopback, up, and every job of the sandbox joins it, so the
  network does not depend on the OCI runtime. Sandboxes with `isolation: container` are placed only on
  workers offering it.
- **LayerCache.** Downloads, verifies and unpacks each layer once per digest; sandboxes pin
  the layers they use, and unpinned layers are evicted least recently used first beyond a size
  budget.
- **Rootfs.** Assembles a sandbox's file system from cached layers: an overlay mount with a
  private upper directory on Linux, a copy elsewhere. Starting a sandbox runs off the
  connection loop; leases for jobs waiting on it keep being renewed.
- **Executor.** One per job kind; runs the job inside a sandbox.
- **Reconciler.** Converges local sandboxes to the `Assignment` the server sends. An
  assignment is derived, never stored: the sandboxes scheduled on that worker, with their specs
  and generations. The gateway pushes it again whenever one of them changes.
- Results are built from the held `Lease`, so every report carries its fencing token. Results
  wait in a local outbox until acknowledged.

## 9. Persistence and events

- Postgres only. An entity's events are its source of truth; loading replays them.
- `streams` holds one row per entity with its version, for optimistic concurrency and listing.
- `events` is the global log and the outbox: unique `(subject, stream_version)` and a gapless
  `sequence` assigned under one advisory lock per commit, so it follows commit order.
- Queries read projection tables written in the same transaction, added when a query needs one.
- Envelope, CloudEvents-aligned: `id`, `type`, `subject`, `time`, `data`, plus `schema_version`,
  `stream_version`, `sequence`, `actor`, `correlation_id`, `causation_id`.
- Delivery is at least once, in global order. Consumers keep a checkpoint and are idempotent,
  which entity methods that ignore satisfied changes make natural. LISTEN/NOTIFY wakes them,
  with polling as a fallback.
- Schemas evolve additively. A breaking change adds a version and an upcaster
  (`impl From<XV1> for XV2`). Snapshot tests freeze serialized forms.

## 10. Interfaces

- **REST** `/v1`: resource-oriented, custom verbs as subresources
  (`POST /v1/sandboxes/{id}/stop`), cursor pagination, label filters, RFC 3339 timestamps,
  RFC 9457 problems with stable `code` values and per-field errors, `Idempotency-Key` on every
  `POST`.
  Routes and their OpenAPI description are registered together in `igloo-control`; the document
  is committed as `schemas/openapi.json` and a test fails on drift.
- **SSE**: live events and logs from the event log, resumable with `Last-Event-ID`.
- **Worker protocol** `igloo.worker.v1`: the server sends the full desired `Assignment`, lease
  grants, cancels and drain; the worker sends hello, heartbeats, status, logs and results.
  The handshake negotiates the version; workers one version behind are accepted.
- **MCP**: streamable HTTP at `/mcp` beside REST, with the same token and resources; tools are
  thin adapters over the command bus, named like their commands (`task.create`). They neither
  approve nor merge. See `docs/mcp.md`.
- **Web console**: a React app in `web/` (TanStack Start in SPA mode, shadcn/ui, Bun), served as
  static files at `/` from `IGLOO_WEB_DIR`. It uses only REST and the event stream, with types
  generated from `schemas/openapi.json`, and performs every human action. See ADR 0012.
- **Repository files**: `.igloo/pipeline.toml` (pipeline spec, JSON Schema in `schemas/`) and
  `.igloo/agents.toml` (protected paths, budgets, task categories).

## 11. Identity and authorization

- Humans authenticate through WorkOS AuthKit; tokens are verified locally.
- Agents are not identity-provider users. Igloo issues short-lived agent tokens naming the
  agent, its human principal and one repository.
- Authorization is Cedar, in process, behind `PolicyEngine`. Every decision takes the actor,
  the command, the resource and its repository.
- Until identity lands, a development bearer token maps to a fixed human actor and the policy
  allows everything.
- Repository secrets are encrypted at rest (XChaCha20-Poly1305, bound to their repository and
  name) under the server's `secrets_key`. Jobs name the secrets they need; the gateway puts the
  values in the lease grant's environment and masks them in job output. Events, logs and
  responses carry names only.
- Workers hold no API credentials. Their assignment carries presigned blob URLs: a keyed
  blake3 hash over method, digest and expiry, signed with the server's `blob_key`. A URL opens
  one blob for one method until it expires, and acts as the gateway.

## 12. The agent loop

The experience the platform builds toward:

1. From the agent in the user's editor (any MCP client), the user asks Igloo to implement
   tasks, for example two spec sections in parallel. The local agent calls Igloo's MCP tools.
2. Each task gets a sandbox forked from a warm snapshot of the repository at a commit: source,
   `.git` and built dependencies. It starts in under a second. Nothing on the user's machine
   changes.
3. An agent harness runs the chosen coding tool headless in the sandbox, with the repository's
   credentials for it. The agent edits, runs checks, iterates and commits.
4. Its work streams as a normalized transcript: messages, file reads and edits, commands and
   their results, the evolving diff.
5. Its commits become a revision of a change. Igloo runs the repository's checks on the
   revision in a fresh sandbox; optionally a second agent reviews it.
6. The user reviews the change in Igloo: diff, transcript, checks, comments. A comment sends the
   agent back for a new revision. Approval merges: Igloo pushes to the default branch.
7. The outcome (task, change, checks, review, merge or revert) is recorded and feeds earned
   autonomy.

**Agent harnesses.** A harness adapts one coding tool: Claude Code, Codex, Pi, others. It runs
the tool headless with a prompt, supplies its credentials (a subscription token or an API key,
stored as repository secrets) and translates its output into transcript events. A tool without
a harness still runs as a plain command, with its raw output as transcript.
`.igloo/agents.toml` names the repository's default tool; a task may choose another.

**Changes and git.** The forge (GitHub first) is the git host and nothing more: Igloo fetches
commits, pushes revision branches and pushes merges. Review, checks and merge decisions live in
Igloo. `igloo change checkout <id>` fetches a change locally as a branch.

## 13. Agents and autonomy

- **Protected paths** always need a human merge and are never auto-merged: sandbox runtimes,
  policy and authentication, migrations, contract surfaces, CI configuration, agent
  instructions, specs and ADRs. Listed in `.github/CODEOWNERS`; enforced by policy once
  Cedar lands.
- **Autonomy stages**, per repository and task category, earned from recorded outcomes:
  - Observe: triage and explain CI failures.
  - Propose: changes for dependency updates and flaky-test repairs.
  - Contribute: scoped tasks; a human reviews everything.
  - Maintain: auto-merge routine categories within budgets.
- **Outcome events**, per repository: task reference, change, review verdict, check results,
  reverts. They drive autonomy stages, evaluations and readiness signals.
- **Break-glass CI**: GitHub Actions builds Igloo independently of Igloo, permanently.
- **Tenancy** is undecided. Nothing global may hold grants, budgets or policy.

## 14. Not built

- A generic resource API server, CRDs or dynamic schemas. Resources are Rust types.
- A watch protocol beyond SSE over the event log.
- A label selector language. Filters are equality.
- Finalizers, admission webhooks, owner-reference garbage collection. Cascades are reactors.
- Consensus or etcd. Postgres is the source of truth; leader election is an advisory lock.
- A scheduler framework. Placement is one pure function.
- Dynamic plugins, full event sourcing, a home-grown workflow engine, microservices.

## 15. Roadmap

Each phase ends with something usable. Phases 1 to 6 target Igloo's own repository;
phase 7 opens Igloo to external repositories.

| Phase                   | Scope                                                                                                                                                             | Exit gate                                                                                                                             |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| 1 Run                   | Core vocabulary, Postgres + outbox, command bus, controllers, worker protocol, `ProcessRuntime`, SDK, CLI                                                         | `igloo run -- cargo test` in a local repository runs on a worker and streams logs and exit code                                       |
| 2 Isolate               | OCI runtime (youki), image import, overlay layer cache, seal and fork, presigned blob URLs, idempotency storage, lease expiry                                     | An imported image runs `cargo test` isolated on Linux; a fork of a warm snapshot starts in under a second; a lost lease fails its job |
| 3 Changes               | `Repo` with secrets and trust settings, the forge as git host, git checkouts in sandboxes, `Change` with revisions, checks run by Igloo, merge, outcome recording | Igloo checks and merges its own changes; GitHub Actions stays as break-glass                                                          |
| 4 Agents                | Agent harnesses (Claude Code, Codex, Pi, ...), normalized transcripts, `Task`, MCP server, agent tokens and grants                                                | From an editor's agent, two Igloo tasks run in parallel; both changes are reviewed in Igloo and one is merged                         |
| 5 Console               | Web console served by the server (ADR 0012), event stream, read models for screens, diffs, worker usage                                                           | Through the tunnel, the user follows and reviews a task and merges its change from the console alone                                  |
| 6 Autonomy              | Autonomy stages, Firecracker, multi-node, workflows                                                                                                               | Igloo reaches the Maintain stage for one task category                                                                                |
| 7 External repositories | Onboarding, more forges and toolchains, tenancy decision                                                                                                          | An external repository runs CI and the agent loop on Igloo                                                                            |
