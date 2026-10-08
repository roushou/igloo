# 0011. Igloo owns the agent loop; coding tools plug in

- Status: accepted
- Date: 2026-10-04

## Context

The loop Igloo exists for (task, agent work, checks, review, merge, outcome) could be stitched
from existing pieces: an agent CLI started by hand, GitHub pull requests for review, GitHub
Actions for checks. That keeps the user switching between tools, splits the record of what
happened across systems, and gives Igloo no say in review or merge. Users also work with
several coding tools (Claude Code, Codex, Pi) and pay for some through subscriptions rather than
API keys.

## Decision

- Igloo owns the loop: tasks, transcripts, changes, checks, review and merge are Igloo
  entities and screens. The forge is the git host and nothing more.
- Coding tools plug in through agent harnesses: run headless, supply credentials (subscription
  token or API key, as repository secrets), translate output into a normalized transcript. Any
  command still runs without a harness.
- The user's own editor agent is the primary entry point, through Igloo's MCP server; the CLI
  and web UI serve the same commands.
- Infrastructure comes first (isolation, warm forks, changes), each phase aimed at this loop.

## Consequences

Igloo needs a change and review model, a web UI and forge integration of its own. The outcome
record is complete in one place. No coding tool is privileged; adding one is a harness.
GitHub Actions remains the independent break-glass CI.

## Alternatives considered

- Review on GitHub pull requests: the record and the decision live outside Igloo.
- One supported agent: ties Igloo to one vendor and one pricing model.
- A thin loop before isolation: throwaway versions of most of the pieces above.
