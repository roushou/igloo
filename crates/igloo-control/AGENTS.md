# igloo-control

The server: a library plus a thin `main.rs`. `app/` (bus, handlers, controllers, reactors,
`Extension`), `platform/` (one module per primitive), `ci/` (the CI product), `ports/`,
`adapters/`, `transport/`. Each module registers itself through one `Extension` impl.

- Every write goes through the `CommandBus`. Transports parse, dispatch and map results.
- `composition.rs` (`Server`) is the only place naming concrete adapters; it can embed a
  worker for development and end-to-end tests. Platform modules never import `ci/`.
- Only composition code names concrete adapters.
- SQL changes require `cargo sqlx prepare --workspace` and a committed `.sqlx/`.
