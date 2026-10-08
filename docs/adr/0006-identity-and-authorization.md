# 0006. Igloo authorizes every actor, scoped to a repository

- Status: accepted
- Date: 2026-10-04

## Context

Humans need SSO. Agents act for humans with narrow, temporary permissions on specific
repositories. Authorization must be fast, testable and deterministic.

## Decision

WorkOS AuthKit authenticates humans behind an `IdentityProvider` port; tokens are verified
locally. Agents receive Igloo-issued, short-lived tokens naming the agent, its human principal
and one repository. Cedar authorizes in process behind `PolicyEngine`, as a layer on the
command bus. Every decision takes the actor, the command, the resource and its repository.
Until identity lands, a development token maps to a fixed human and the policy allows
everything.

## Consequences

No network call per request for authentication or authorization. Agent permissions are
explicit, per repository and expiring. Repository scoping leaves room for tenancy.

## Alternatives considered

- A hosted authorization service: a network round trip in every decision.
- Agents as identity-provider users: the wrong model for ephemeral, scoped actors.
- Global grants: one misbehaving repository would force revoking access everywhere.
