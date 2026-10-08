# igloo-worker

The worker library and binary. Talks to the server only through `igloo-worker-protocol`;
never depends on `igloo-control` or `igloo-api`. `igloo-control` embeds it in development. Converges local sandboxes to the received
`Assignment` using the pure `plan` from `igloo-core`.

- `SandboxRuntime` implementations live in `runtime/`. `ProcessRuntime` has no isolation and
  is refused outside development mode.
- `unsafe` is denied workspace-wide; exceptions only in `runtime/`, each with a `// SAFETY:`
  comment.
- Every result is built from its `Lease`, so it carries the fencing token.
