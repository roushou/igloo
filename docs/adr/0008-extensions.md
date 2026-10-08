# 0008. Modules register themselves through `Extension`

- Status: accepted
- Date: 2026-10-04

## Context

The server grows by modules: sandboxes, snapshots, jobs, workers, CI, later agents. Each brings
commands, controllers, reactors and routes. Registering all of them by hand in `main.rs`
turns startup into one function that knows everything.

## Decision

Each module implements `Extension` once, registering its commands, controllers, reactors and
routes into the `PlatformBuilder`. `main.rs` lists the extensions to install. Extensions are
compiled in and run in process. Platform modules never import product modules.

## Consequences

Adding a module touches its own code and one line in `main.rs`. Tests install only the
extensions they need.

## Alternatives considered

- Manual registration in `main.rs`: one growing function coupled to every module.
- Dynamic plugins (shared libraries, WASM): ABI and versioning cost with no current need.
- A crate per product: see ADR 0002.
