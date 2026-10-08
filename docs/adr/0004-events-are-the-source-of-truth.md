# 0004. Events are the source of truth, committed in global order

- Status: accepted
- Date: 2026-10-04

## Context

Igloo needs an audit trail, a trigger for controllers and reactors, and a stream for clients.
Entities must load cheaply, queries must stay cheap, and entity state must not become a second
serialized contract next to the events.

## Decision

An entity's events are its source of truth. Loading replays them; streams are short (a handful
to a few hundred events). A `streams` row per entity holds its version for optimistic
concurrency. Queries read projection tables, updated in the same transaction as the events and
added only when a query needs them. Every commit takes one transaction-level advisory lock
before appending, so the global `sequence` is gapless and increases in commit order. The log
doubles as the outbox; envelopes use CloudEvents field names. Delivery is at least once, in
global order; each consumer keeps a checkpoint and is idempotent.

## Consequences

No lost or phantom events, and no consumer ever skips a slower transaction's event. Only
events need schema evolution: additive changes, upcasters for breaking ones, snapshot tests.
Commits are serialized, which bounds write throughput; acceptable for one control plane, and
revisited with snapshots or partitioned sequences if it ever binds.

## Alternatives considered

- A serialized state table per entity: entity state becomes a second contract with its own
  migrations.
- Publishing after commit: events lost on crash.
- A plain `BIGSERIAL` sequence: concurrent transactions commit out of order, so readers skip
  events permanently.
