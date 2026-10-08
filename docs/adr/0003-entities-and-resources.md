# 0003. Entities record events, resources converge

- Status: accepted
- Date: 2026-10-04

## Context

Domain logic must be deterministic and testable without I/O. Infrastructure state must
survive crashes, missed messages and partitions.

## Decision

An entity is built with `new` and changed through its methods. Each method either rejects the
change or records events that are applied to the state immediately; the store commits state
and events together, and `replay` rebuilds the state from history. Methods are pure: time and
IDs are arguments. `Resource: Entity` adds `spec`, `status`, `labels`, `generation` and a pure
`plan`. A generic `Controller<R>` drives each resource kind level-triggered: it re-plans on
every relevant event and on a periodic resync, and a `Reconciler<R>` executes the plan's
actions idempotently.

## Consequences

Given-when-then tests cover every command and plan branch. Generic handlers and controllers
cover most code paths. Effects across entities go through events.

## Alternatives considered

- Services mutating state directly: logic scattered and untestable without I/O.
- A decider with a command enum and one `decide` function: a dispatcher in place of methods on
  the type that owns the state.
- Edge-triggered controllers: state lost on a missed event.
- A Kubernetes-style generic API server: dynamic schemas and machinery Igloo does not need.
