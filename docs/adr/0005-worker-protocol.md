# 0005. Workers dial one bidirectional gRPC stream

- Status: accepted
- Date: 2026-10-04

## Context

Workers may sit behind NAT, run on different versions and must never accept inbound
connections. Assignments, status and logs flow both ways with low latency.

## Decision

Package `igloo.worker.v1`. The worker dials out and holds one bidirectional stream,
authenticated by a join token in development and mutual TLS in production. The server sends
the full desired `Assignment`, lease grants with fencing tokens, cancels and drain. Bulk bytes
move through `BlobStore` URLs. The handshake negotiates the version; workers one version
behind are accepted. Results wait in a local worker outbox until acknowledged.

## Consequences

No inbound ports on workers. A stale worker cannot overwrite results. `buf lint` and
`buf breaking` gate changes; removed fields are `reserved`.

## Alternatives considered

- Server calls workers: needs inbound access to every worker.
- HTTP polling: higher latency and load, awkward log streaming.
