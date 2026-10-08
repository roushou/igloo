# 0002. Crates earn their existence and own their concern

- Status: accepted
- Date: 2026-10-04

## Context

The server and the worker differ in privileges, platform and release cadence, yet must agree
on a protocol. The SDK must not compile server code. Splitting code into crates for its own
sake produces crates that cannot stand alone and glue crates that only wire others together.

## Decision

A crate exists only for a deployable binary, a published library, a contract shared by
separate programs, or isolation of heavy or platform-specific dependencies. Everything else is
a module. A crate owns its concern: it declares what it needs as traits it owns, exposes types
it owns, and never reaches into another crate's internals.

Seven crates: `igloo-core` (pure domain shared by server and worker), `igloo-api` and
`igloo-worker-protocol` (contracts), `igloo-control` (server, library plus binary),
`igloo-worker` (worker, library plus binary), `igloo-rs` (SDK, library `igloo`), `igloo-cli`
(binary `igloo`). Products (CI, later agents) are modules of `igloo-control`.

`igloo-control` is the composition point and may depend on `igloo-worker`: it embeds a worker
in development mode and in end-to-end tests. `igloo-worker` never depends on `igloo-control`;
it reaches the server only through the protocol. In production they run as separate
processes under separate users. The CLI is a thin binary over the SDK; workflows such as
running a directory live in the SDK.

## Consequences

The compiler enforces protocol agreement between server and worker. Module boundaries inside
`igloo-control` are kept by convention and review. End-to-end tests live in `igloo-control`
and run the real server with an embedded worker. One command runs Igloo locally.

## Alternatives considered

- A crate per product with a composition binary: crates without standalone value plus a glue
  crate.
- Forbidding any dependency between server and worker: forces a test-only crate to compose
  them, and rules out a single-process development mode.
- Custom tooling to check the dependency graph: duplicates the manifests and adds no safety
  the compiler lacks.
- Contracts inside each block: one block would depend on the other.
