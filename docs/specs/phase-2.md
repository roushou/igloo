# Phase 2: Isolate

Exit gate, on a Linux worker:

1. `igloo run --base <imported rust image> -- cargo test` runs in an isolated container: its
   own root file system, no access to the host's files, no network unless the sandbox allows
   it, CPU and memory bounded.
2. `igloo snapshot <sandbox>` after `cargo build` seals a warm snapshot; a sandbox forked from
   it starts in under a second and runs `cargo test` without recompiling dependencies.
3. A worker killed mid-job loses its lease: the job fails with `lease_lost`, and a result the
   worker reports later is refused.
4. A create retried with the same `Idempotency-Key` returns the original resource.

Order: P2.1 -> P2.2 -> P2.3 -> P2.4 -> P2.5; P2.6 in parallel with P2.3 to P2.5; P2.7 last.

## Groundwork carried from phase 1

- Materialization blocks the worker's connection loop; heartbeats stall during large
  downloads (P2.3).
- Workers download blobs with the REST development token (P2.2).
- An expired lease is never re-offered or failed; its job stays leased forever (P2.6).

## P2.1 Snapshot format v2 and image import

- Contract changes: snapshot manifest format, `igloo-api` (import), `schemas/openapi.json`

### Deliverables

- Manifest v2 (`igloo.snapshot.v2`): each layer is a digest plus a media type, `tar` or
  `tar+gzip`, so OCI layers are stored as published. v1 manifests stay readable.
- Run layers are rooted at `workspace/`; jobs run with `/workspace` as working directory, so a
  working tree layers cleanly over a base image.
- `POST /v1/snapshots/import { "image": "docker.io/library/rust:1.99" }`: the server pulls the
  image's layers from its registry (anonymous token flow, public images) into the blob store and
  registers a snapshot. `igloo snapshot import <image>`.
- `Snapshot` keeps its parent's layers: a run snapshot is `base layers + working tree layer`.
  `igloo run --base <snapshot>`.

### Acceptance

- Importing a small public image registers a snapshot whose layers are the image's, in order;
  a second import stores nothing new.
- A v1 and a v2 manifest both materialize.

## P2.2 Presigned blob URLs

- Contract changes: `proto/` (assignment carries URLs), worker configuration

### Deliverables

- `BlobUrls`: HMAC-signed, expiring `GET` and `PUT` URLs for one digest, verified by the blob
  routes without a bearer token. The signing key comes from configuration.
- The assignment carries a download URL per layer; workers lose `api_token`.

### Acceptance

- A valid URL downloads; an expired, altered or other-digest URL is refused with 403.
- A worker with no API token materializes a sandbox.

## P2.3 Layer cache and overlay materialization

- Contract changes: none

### Deliverables

- Each layer is unpacked once into a per-digest cache directory, handling OCI whiteouts.
- On Linux, a sandbox's root is an overlay mount: cached layers as lower directories, a private
  upper directory. Elsewhere, layers are copied into the sandbox, as today.
- Materialization runs off the connection loop; heartbeats and results keep flowing.
- Cache eviction by least recent use once a configured size is exceeded, never for layers in
  use.

### Acceptance

- Two sandboxes from one snapshot share cached layers and do not see each other's writes.
- Heartbeats keep their cadence while a large snapshot materializes.

## P2.4 OCI runtime

- Contract changes: worker configuration

### Deliverables

- `OciRuntime`: writes an OCI runtime bundle (root, mounts, env, `/workspace` cwd, cgroup v2 CPU
  and memory limits, a fresh network namespace with loopback only for `DenyAll`, the host's for
  `AllowAll`) and drives a runtime binary (`youki` by default; `crun` and `runc` accepted). Each
  sandbox runs a minimal init; jobs run through the runtime's `exec`, streaming stdio.
- Capabilities advertise `oci`; placement requires it for sandboxes that ask for isolation.
- Linux integration tests behind `IGLOO_TEST_OCI=1`; a CI job on Linux installs youki and runs
  them.

### Acceptance

- A job sees only the sandbox's file system, cannot reach the network under `DenyAll`, and is
  killed when it exceeds its memory limit.
- Stopping the sandbox removes its container and mounts.

## P2.5 Seal and fork

- Contract changes: `proto/` (seal request and report), `igloo-api`, `schemas/openapi.json`

### Deliverables

- `POST /v1/sandboxes/{id}/snapshot`: the worker packs the sandbox's upper directory as a layer
  (deletions as OCI whiteouts), uploads it through a presigned URL, and the server registers a
  snapshot of the parent's layers plus that layer. `igloo snapshot <sandbox>`.
- Placement prefers a worker that already caches a sandbox's layers.

### Acceptance

- A fork of a sealed snapshot sees the sealed files and deletions.
- With layers cached, a fork is running in under a second.

## P2.6 Idempotency and lease expiry

- Contract changes: migrations

### Deliverables

- Idempotency storage behind the existing layer: `(actor, key) -> request hash, response`.
  Same key and body replays the response; same key, other body -> 422
  `idempotency.key_reused`. Keys expire after 24 hours.
- `Job` becomes a resource: a leased or running job whose lease expired plans `FailLeaseLost`;
  its controller fails it with `lease_lost`. Retrying is the caller's decision.

### Acceptance

- Scenario tests for the job plan; HTTP tests for replay and key reuse.

## P2.7 Lease loss end to end

- Contract changes: none

### Deliverables

- End-to-end tests with a short lease TTL: a worker cancelled mid-job leaves the job failed
  with `lease_lost` after the TTL; the same worker reconnecting and reporting its result is
  refused; a second worker never receives the job twice.
- `docs/dev-linux.md`: running a Linux worker in a VM (OrbStack or Lima) against a server on
  macOS.

### Acceptance

- The exit gate, items 3 and 4, as tests; items 1 and 2 on the Linux CI job.

## Open decisions

1. **Runtime binary over the `libcontainer` crate.** Driving `youki`/`crun`/`runc` through OCI
   bundles is how containerd works, keeps the worker free of in-process namespace code, and lets
   the runtime be swapped. `libcontainer` would avoid an external binary but couples the worker
   to its internals.
2. **A lost lease fails the job** rather than requeueing it, since jobs may have side effects.
   Automatic retries can come later as a job policy.
3. **`AllowAll` shares the host network** for now; per-sandbox virtual networking waits until a
   product needs it.
