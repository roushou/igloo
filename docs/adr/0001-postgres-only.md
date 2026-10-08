# 0001. Postgres is the only database

- Status: accepted
- Date: 2026-10-04

## Context

sqlx compile-time checked queries are tied to one database. Igloo needs `SKIP LOCKED` for its
job queue, `LISTEN/NOTIFY` for wake-ups and `JSONB` for nested specs.

## Decision

Postgres everywhere: development, tests, production. It holds entity state, the event log,
the job queue and consumer checkpoints. Tests use `sqlx::test` or testcontainers; development uses
`dev/compose.yaml`.

## Consequences

Running Igloo always needs Postgres. Queries are checked at compile time and `.sqlx/` is
committed so builds work offline. No separate queue or broker until load proves the need.

## Alternatives considered

- SQLite first: a second set of queries and migrations for no lasting gain.
- A dedicated queue (Redis, NATS) from the start: another system to run before it is needed.
