# 0010. Build what is expensive to retrofit, deliver progressively

- Status: accepted
- Date: 2026-10-04

## Context

Igloo must last, but it must also be usable early so it can be tested on real repositories
and start building itself. Building for today only paints the design into corners; building
everything up front delays feedback and buries the code in machinery.

## Decision

- Decide and build now what is expensive to change later: identifiers, event envelope and
  actor, entity shape, desired versus observed state, state and events in one transaction,
  ports for every side effect, contract versions, composition seams.
- Defer what is cheap to add later behind an existing seam: policy engines, idempotency
  storage, external buses, multi-node, workflow engines, alternative runtimes.
- Each phase ends with something usable on real repositories.
- Prefer standard tools over custom enforcement; add a custom check only after a real
  violation.

## Consequences

Seams exist before their implementations. Every abstraction must name a second use or a seam
it serves. Phases are vertical slices, not layers.

## Alternatives considered

- Build only what today's task needs: retrofits of IDs, events and boundaries later.
- Build the full platform before first use: no feedback until it is too late to change.
