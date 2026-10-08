# 0007. The command bus is the only write path

- Status: accepted
- Date: 2026-10-04

## Context

REST, MCP, the CLI and reactors all change state. Agents are first-class callers, so
authorization, audit, idempotency and limits must apply identically on every path.

## Decision

Every state change is a typed `Command` dispatched through the `CommandBus`. Handlers are
registered once; cross-cutting concerns are tower layers on the bus (tracing, timeout,
authorization, idempotency). Transports parse, dispatch and map results, nothing more.

## Consequences

One choke point to authorize, audit and rate-limit. New transports cost only parsing. MCP
tool names come from `Command::NAME`.

## Alternatives considered

- Handlers called directly from each transport: concerns duplicated per transport.
- A message broker between transport and handlers: latency and operations for no benefit in
  one process.
