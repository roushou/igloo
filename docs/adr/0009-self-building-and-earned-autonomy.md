# 0009. Self-building Igloo and earned autonomy

- Status: accepted
- Date: 2026-10-04

## Context

Igloo exists to run one loop for any repository: a specified task, an agent working in a
forked sandbox, checks, a change, review, merge. Igloo itself is the first repository it
must build this way. Agents changing the system that runs agents can break the only way to fix
it, and autonomy granted globally cannot be earned or revoked precisely.

## Decision

- Igloo builds itself with the same loop it offers every repository.
- Igloo is the first and, until external repositories are supported, the only repository.
  It is configured like any repository: through `Repo`, `.igloo/pipeline.toml` and
  `.igloo/agents.toml`. The platform and products contain no Igloo-specific logic.
- The repository is the trust boundary: grants, budgets, protected paths and autonomy stage
  attach to `Repo`, configured in `.igloo/agents.toml`, never globally.
- Protected paths always need a human merge and are never auto-merged.
- Autonomy is earned per repository and task category from recorded outcomes: Observe,
  Propose, Contribute, Maintain.
- Outcomes are recorded as events for every repository: task reference, change, review
  verdict, CI results, reverts.
- GitHub Actions builds Igloo permanently, independently of Igloo.
- Tenancy is undecided; nothing global may hold grants, budgets or policy.

## Consequences

Every trust decision takes a repository. Outcome events are a product contract. Igloo's CI
runs twice once Igloo runs its own CI, in exchange for an independent recovery path.

## Alternatives considered

- Supporting external repositories from the start: tenancy, onboarding and toolchain variety
  before the loop is proven on one repository.
- Special-casing Igloo for speed: every shortcut becomes a retrofit when external
  repositories arrive.
- Retiring GitHub Actions after dogfooding: a broken Igloo could not build its fix.
- Autonomy by configuration: no evidence behind each step up.
- Protected paths enforced only by policy: unprotected until policy exists.
