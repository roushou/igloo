# Igloo: agent operating manual

Igloo is a self-hosted platform where humans and agents build and maintain software. Rust
workspace, with a React web console in `web/` (ADR 0012). `CLAUDE.md` imports this file.

## Read before any change

1. This file.
2. `docs/architecture.md`: what exists and where it lives.
3. `docs/conventions.md`: how code is written. Not optional.
4. The ADRs in `docs/adr/` that touch your area.
5. Your task in `docs/specs/`.

## Workspace map

| Crate                   | Kind      | Purpose                                                | May depend on (internal)                |
| ----------------------- | --------- | ------------------------------------------------------ | --------------------------------------- |
| `igloo-core`            | lib       | Pure domain: vocabulary and platform primitives        | nothing                                 |
| `igloo-api`             | lib       | Public API contract: REST types, problems, OpenAPI     | core                                    |
| `igloo-worker-protocol` | lib       | Server <-> worker gRPC contract                        | core                                    |
| `igloo-control`         | lib + bin | Server: bus, controllers, ports, adapters, inbound, CI | core, api, worker-protocol, worker, git |
| `igloo-worker`          | lib + bin | Worker: runtimes, snapshots, executors, reconciler     | core, worker-protocol                   |
| `igloo-rs`              | lib       | Rust SDK, library name `igloo`                         | api                                     |
| `igloo-cli`             | bin       | CLI, binary `igloo`                                    | rs, core, git                           |
| `igloo-git`             | lib       | Git client over the `git` binary                       | core                                    |

## Commands

| Command                                                 | When                                                                                       |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| `cargo fmt && dprint fmt`                               | Before finishing. Formats Rust, TOML, Markdown, JSON.                                      |
| `cargo clippy --workspace --all-targets -- -D warnings` | After every change.                                                                        |
| `cargo nextest run --workspace`                         | After every change.                                                                        |
| `cargo shear` and `cargo deny check`                    | After changing dependencies.                                                               |
| `buf lint`                                              | After changing `proto/`.                                                                   |
| `UPDATE_SCHEMAS=1 cargo nextest run -p igloo-control`   | After changing API types. Rewrites `schemas/`.                                             |
| `docker compose -f dev/compose.yaml up -d --wait`       | Start local Postgres (`postgres://igloo:igloo@localhost:5432/igloo`).                      |
| `cargo sqlx prepare --workspace`                        | After changing any SQL query, with `DATABASE_URL` set and migrations run. Commit `.sqlx/`. |

CI (`.github/workflows/ci.yml`) runs `buf lint`, the format checks, clippy, nextest (Postgres tests use
testcontainers, so Docker must run), doc tests, the sqlx schema check, `cargo deny check` and
`cargo shear`.

## Hard rules

1. `igloo-worker` never depends on `igloo-control`; it talks to the server only through the
   protocol. `igloo-control` may embed the worker (development mode, end-to-end tests).
   Platform modules never import product modules (`ci/`, later agents).
2. `igloo-core` performs no I/O and has no async runtime.
3. Contract crates hold data and conversions only.
4. Never read the clock, generate IDs or spawn tasks directly. Use `Clock`, `IdGenerator`
   and `TaskSupervisor`. Clippy enforces this.
5. Every concern has an owning type. No free functions carrying behavior.
6. Cross boundaries with `From`, `TryFrom`, `FromStr`, `Display`. Wire types never enter the
   application layer.
7. No `unwrap`, `expect` or `panic!` outside tests. `anyhow`/`eyre` only in a binary's
   `main.rs`.
8. One transaction touches one entity. Cross-entity effects go through events.
9. Every command and every `plan` branch has a scenario test. Every port has a memory adapter
   that passes the port's conformance suite.
10. Contract surfaces change only when the task spec says so: `proto/`, `schemas/`,
    `crates/*/migrations/`, `crates/igloo-api`, `crates/igloo-worker-protocol`,
    `crates/igloo-control/src/ports`. Otherwise stop and report.
11. Migrations are forward-only. Never edit a merged migration.
12. A new crate, port, core trait or external dependency needs approval. Propose it in your
    report with the reason.
13. Never weaken a test, lint or check. Every `#[allow(...)]` carries a reason.
14. Trust settings (grants, budgets, policy, protected paths) belong to a `Repo`, never to
    global configuration.
15. No Igloo-specific logic in the platform or products. Igloo is configured like any
    repository, through `Repo` and its `.igloo/` files.
16. Comments and docs state what code does and guarantees. Rationale and history go in ADRs.
17. Implement only the task. Put follow-ups in your report.

## Definition of done

- Every acceptance criterion in the task is met and covered by a test.
- Every CI step passes locally.
- Every public item has a doc comment stating its invariants.
- Your final message is a short report: Summary, Contract changes, New dependencies,
  Follow-ups, Open questions. A line or two each.

## When something is ambiguous

Stop and ask, offering concrete options. Never guess on contracts, public type names, or
anything in hard rule 10.
