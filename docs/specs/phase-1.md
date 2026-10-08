# Phase 1: Run

Exit gate: with Postgres, `igloo-control` and one `igloo-worker` running locally,
`igloo run -- cargo test` inside any local repository uploads the working tree as a snapshot,
runs the command in a sandbox on the worker, streams its output, exits with the command's
exit code, and leaves the corresponding events in the `events` table.

Order: P1.1 -> P1.2 -> P1.3 -> P1.4 and P1.5 (parallel) -> P1.6 -> P1.7 -> P1.8 -> P1.9.

## P1.1 Core vocabulary

- Depends on: nothing
- Contract changes: `igloo-core` public API (new)
- Read first: conventions 6 to 9; ADR 0003, 0010

### Deliverables (`igloo-core` root vocabulary, `igloo_core::testing`)

- `Prefixed`, `Id<T>`, `IdError`, following the TypeID specification.
- `Digest` (`blake3:<64 hex>`), `Timestamp` (no `now()`), `Version`, `Generation`.
- `Actor`: `Human { user: UserId }`, `Agent { agent: AgentId, principal: UserId }`,
  `System { component: SystemComponent }`.
- `Labels`: at most 64 entries; keys `[a-z0-9]([a-z0-9._/-]{0,62}[a-z0-9])?`, values up to
  256 characters.
- `ErrorCode`, `ValidationErrors`, `Validator`.
- `Entity`, `Resource`, `Plan<A>`, `Event` as in conventions section 9.
- `Scenario<E>` (feature `testing`): `given`, `create`/`try_create`, `when`/`try_when`, `then`,
  `then_no_events`, `then_error(code)`, and for resources `plan`, `then_actions`,
  `then_converged`, `then_recheck`. Fixed clock.

### Acceptance

- The TypeID specification's valid and invalid test vectors pass.
- Property test: any UUID round-trips through `Id<T>` and through `Digest`.
- `Labels` and `Validator` report every invalid field, not the first.
- A private toy entity in `#[cfg(test)]` exercises every `Scenario` method; failures print
  expected and actual values.
- insta snapshots of the JSON forms of `Id`, `Digest`, `Timestamp`, every `Actor` variant and
  `Labels`.

## P1.2 Platform primitives

- Depends on: P1.1
- Contract changes: `igloo-core` public API

### Deliverables (`igloo_core::{snapshot, sandbox, worker, job}`)

- `SnapshotManifest` (ordered layer digests) and `SnapshotId` (digest of the manifest).
- `Sandbox` resource (prefix `sbx`):
  - Spec: `snapshot`, `desired` (`Running | Stopped`), `limits` (`ResourceLimits`), `network`
    (`DenyAll` default, `AllowAll`), `env` (keys `[A-Z_][A-Z0-9_]*`, at most 128), `labels`.
  - Status: `phase` (`Pending`, `Scheduled { worker }`, `Starting`, `Running`, `Stopping`,
    `Stopped`, `Failed { reason }`) and `observed_generation`.
  - Commands: `Stop` (idempotent), `RecordStatus` (rejects stale generations and leaving a
    terminal phase), `Schedule { worker }`.
  - `plan`: `Pending` with desired `Running` -> `Act([Place])`; otherwise `Converged`, since
    the worker converges a placed sandbox and its reports wake the controller.
- `Worker` resource (prefix `wrk`): `Capabilities` (OS, architecture, runtimes, protocol
  version), heartbeats, `Draining`; `plan` marks it `Lost` after missed heartbeats.
- `Job` entity (prefix `job`): `JobSpec::Execute { sandbox, argv, env, timeout }`; phases
  `Queued`, `Leased`, `Running`, `Succeeded { exit_code }`, `Failed { reason }`, `Cancelled`.
  `Lease { worker, fencing_token, expires_at }`; results with a stale token are rejected.
- `Capabilities::satisfies(&Requirements)` returning every mismatch.
- Error codes: `sandbox.invalid_transition`, `sandbox.stale_generation`,
  `sandbox.invalid_spec`, `job.stale_lease`, `job.invalid_transition`.

### Acceptance

- Scenario tests for every command path, idempotent case, rejection and `plan` branch.
- Property tests: a terminal sandbox phase is never left; fencing tokens only increase.
- insta snapshots of every event variant.

## P1.3 Application layer

- Depends on: P1.2
- Contract changes: `crates/igloo-control/src/ports` (new)
- Read first: conventions 10 to 16; ADR 0007, 0008

### Deliverables (`igloo_control::{app, ports, adapters::memory}`)

- Ports: `Clock`, `IdGenerator` (+ `IdGeneratorExt`), `EntityStore<E>`, `EventLog`,
  `Checkpoints`, `PolicyEngine`; `TaskSupervisor`. `JobQueue`, `BlobStore` and `LogStore` are
  defined with their first users (P1.4 to P1.6).
- Application layer: `Command`, `CommandHandler`, the generic `EntityHandler`, `RequestContext`,
  `AppError`.
- `CommandBus` with layers: tracing, timeout (default 10 s), authorization through
  `PolicyEngine`, idempotency (passes through; storage arrives in phase 2).
- `Controller<R>` and `Reconciler<R>`; `Reactor` with a checkpointed runner.
- `Extension`, `PlatformBuilder`, `Platform`.
- Memory adapters for every port, `AllowAllPolicy`, and a conformance suite per storage port.

### Acceptance

- A toy entity's creation and command round-trip through the bus with memory adapters.
- An unregistered command returns a typed error; a slow handler returns `command.timeout`;
  a stale commit returns `AppError::Conflict`; a denied command returns `Forbidden`.
- A controller converges a toy resource, requeues on `Recheck` and backs off on error.
- A reactor processes a redelivered event once.
- Memory adapters pass their conformance suites.

## P1.4 Postgres adapters

- Depends on: P1.3
- Contract changes: `crates/igloo-control/migrations/` (new)
- Read first: ADR 0001, 0004; architecture section 9

### Deliverables

- Migrations: `streams`, `events`, `checkpoints`.
- `PgDatabase` (pool plus migrations), `PgEntityStore<E>` replaying events, `PgEventLog` with
  LISTEN/NOTIFY and polling, `PgCheckpoints`.
- `.sqlx/` committed; CI runs `cargo sqlx prepare --workspace --check` against a Postgres
  service.
- Job, log and blob storage arrive with their first users (P1.5 to P1.7).

### Acceptance

- Every Postgres adapter passes the same conformance suites as the memory adapters, against
  testcontainers Postgres.
- A failed commit leaves neither the stream row nor its events.
- Concurrent commits produce gapless sequences in commit order.

## P1.5 Platform modules

- Depends on: P1.3
- Contract changes: none

### Deliverables (`igloo_control::platform`)

- `SandboxModule`, `WorkerModule`, `JobModule`: commands, handlers, controllers, reactors.
- Placement: a pending sandbox is scheduled on the first schedulable worker meeting its
  requirements; with none, the controller retries with backoff.
- Assignments are derived, not stored: `SandboxQueries::on_worker` is a worker's assignment,
  which the gateway pushes (P1.7).
- Worker liveness: a worker disconnected past its grace period is marked lost, and its
  unfinished sandboxes fail with `WorkerLost`.
- Jobs: submission checks the sandbox is live; `JobQueries::queued_for(worker)` lists the jobs a
  worker should lease; a sandbox that ends cancels its unfinished jobs.
- Snapshots arrive with blob storage (P1.6).

### Acceptance

- With memory adapters: a created sandbox is placed on a capable worker; without one it stays
  pending until a worker registers; stopping a sandbox updates its worker's assignment; a lost
  worker fails its sandboxes; jobs queue for their sandbox's worker and are cancelled when it
  ends; submitting to an unknown or ended sandbox is rejected.

## P1.6 REST, snapshots, blobs

- Depends on: P1.4, P1.5
- Contract changes: `igloo-api` (new), `schemas/openapi.json` (new)
- Read first: architecture section 10

### Deliverables

- `BlobStore` port with memory and filesystem adapters; blobs are verified against their
  blake3 digest. A snapshot's manifest is stored as the blob under its id.
- `SnapshotModule`: `StoreBlob`, `RegisterSnapshot`; creating a sandbox requires its snapshot.
- `igloo-api` types and conversions: sandboxes, jobs, snapshots, `Problem`.
- Routes: `PUT|GET /v1/blobs/{digest}`, `POST /v1/snapshots`, `GET /v1/snapshots/{id}`,
  `POST|GET /v1/sandboxes`, `GET /v1/sandboxes/{id}`, `POST /v1/sandboxes/{id}/stop`,
  `POST /v1/sandboxes/{id}/exec`, `GET /v1/jobs/{id}`.
- Development bearer token; `Idempotency-Key` validated and carried in `RequestContext`.
- OpenAPI drift test regenerating with `UPDATE_SCHEMAS=1`.

### Acceptance

- HTTP tests for every route, success and error; a blob that does not match its digest is
  rejected; unknown resources are 404 with `<entity>.not_found`; repeated stop is 202.
- insta snapshot of a validation problem listing every invalid field.

## P1.7 Worker protocol, gateway, logs

- Depends on: P1.5, P1.6
- Contract changes: `proto/igloo/worker/v1/` (new), `igloo-worker-protocol` (new), migrations
- Read first: ADR 0005

### Deliverables

- `worker.proto`: one `Connect` stream. Worker: hello, heartbeat (renews held leases), sandbox
  status, job started, log chunk, job result. Server: welcome, full assignment, lease grant,
  cancel, result acknowledgement. `buf lint` in CI; generated with protox, no `protoc`.
- `igloo-worker-protocol`: generated types, client and server, conversions to and from core.
- Gateway (`igloo_control::transport::gateway`): join-token authentication, version check,
  registration or reconnection, then one session per connection that pushes the assignment
  and lease grants on every log change, re-grants live leases after a reconnect, cancels
  revoked jobs, turns reports into commands, and records the disconnection.
- `LogStore` port (memory, Postgres) deduplicating chunks by job, stream and byte offset;
  `GET /v1/jobs/{id}/logs` as server-sent events, resumable with `Last-Event-ID`.
- `TaskSupervisor::spawner` for tasks started on demand.

### Acceptance

- A client over TCP registers, receives its assignment, reports status, is granted a lease,
  streams output (resent chunks stored once), renews and reports a result that is
  acknowledged; a stale-token result is refused; logs replay over SSE; a cancelled job is
  cancelled on its worker; a disconnect is recorded; a wrong join token, an unsupported
  version and an unknown worker id are refused.

## P1.8 Worker

- Depends on: P1.7
- Contract changes: none

### Deliverables (`igloo-worker`)

- `WorkerConfig` from `IGLOO_WORKER_*`, reporting every problem; the process runtime is refused
  without `dev_mode`.
- Gateway connection with reconnect backoff; the worker id is kept across restarts and
  dropped when the server no longer knows it.
- `Sandboxes` converging local sandboxes to the assignment; sandboxes recorded before a
  restart are adopted, not recreated.
- `SandboxRuntime` with `ProcessRuntime`; `SnapshotCache` downloading, verifying and caching
  blobs and unpacking tar layers; `BlobSource` over the REST API.
- `Execution` running a leased job, streaming output and reporting the exit code or timeout;
  results stay in an on-disk outbox until acknowledged; running jobs survive reconnections.

### Acceptance

- Against a scripted gateway: a job runs in a materialized sandbox and its output and exit code
  arrive; a timeout is reported; a result is resent after a reconnect until acknowledged; a
  restarted worker adopts its sandbox without downloading it again; a cancelled job reports
  no result. The real server is exercised end to end in P1.9.

## P1.9 Binary, SDK, CLI

- Depends on: P1.6, P1.8
- Contract changes: none

### Deliverables

- `igloo-control` binary: `RawConfig` -> `Config` (`IGLOO_*`), `Server` (composition: Postgres
  adapters, filesystem blobs, REST and gateway listeners, graceful shutdown on SIGINT and
  SIGTERM), and `IGLOO_EMBEDDED_WORKER=true` running a worker in the same process.
- `igloo-rs`: `Client` for blobs, snapshots, sandboxes, jobs and SSE logs; `Layer::from_dir`
  packing a directory as a deterministic tar honouring `.gitignore`; `Client::run`.
- `igloo-cli`: `igloo run [--label k=v] [--timeout s] [--keep] -- <argv>` exiting with the
  command's exit code, `igloo sandbox create|get|list|stop`, `igloo logs <job>`.

### Acceptance

- Invalid configuration reports every problem at once.
- The exit gate, in `igloo-control/tests/`: Postgres from testcontainers, the server with an
  embedded worker, `Client::run` on a directory with a `.gitignore`; output, exit code and the
  events in Postgres are checked.
