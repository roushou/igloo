# 0012. A web console served by the server

- Status: accepted
- Date: 2026-10-09

## Context

Igloo's state is reachable only through CLI commands and MCP tools, one resource at a time. Following
a task from goal to merge takes a dozen commands and copied ids; a stalled run and a working one look
the same; failure reasons sit in job logs. Igloo owns review (ADR 0011), so it needs a screen for it.

The server runs on one host and listens on localhost, reached through an SSH tunnel and later a
tailnet. Its only credential today is the development bearer token.

## Decision

- A web console in `web/`: React, TanStack Start in SPA mode (TanStack Router and Query), shadcn/ui
  over Tailwind CSS, Bun as runtime and package manager, Biome for lint and format, Vitest for
  tests. No browser end-to-end tests while the console is young: they cost more than they catch.
- The console is static files. `igloo-control` serves them at `/` from `IGLOO_WEB_DIR`, with an SPA
  fallback, on the same origin as `/v1` and `/mcp`. No Start server functions and no second
  process.
- The console uses only the public REST API and its event stream. Its types are generated from
  `schemas/openapi.json`; the console never reads the database or calls internal routes.
- Live state comes from one server-sent event stream over the event log, resumable by sequence.
  Each event names the resource it changed; the console refetches that resource.
- The console performs every human action the API offers, approval and merge included. It
  authenticates with the bearer token until identity lands (ADR 0006); the token stays in the
  browser, which is acceptable only while the server is reachable on localhost or a tailnet.

## Consequences

The server gains a static file service, an event stream and read endpoints shaped for screens
(filters, merge readiness, diffs, worker usage); each is a contract change. Deploying builds `web/`
with Bun next to the binaries. CI gains a web job; Igloo's own pipeline gains Bun in its warm
snapshot. TypeScript enters the repository, confined to `web/`.

## Alternatives considered

- SvelteKit: equivalent; React with shadcn/ui and TanStack was the owner's choice.
- A Start server or Node process in front of the API: a second service to deploy and secure for no
  gain while the API is complete.
- Server-rendered HTML from axum: every live view written by hand.
- Polling instead of an event stream: slower, and heavier on the server as screens multiply.
