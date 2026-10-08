# Working with agents

For the human owner. Agents read `AGENTS.md`.

## Roles

- The owner decides: contracts, core traits, ports, `proto/`, `schemas/`, migrations, ADRs,
  task specs, and every merge.
- Claude Code and Codex implement and review. Alternate per task: one implements, the other
  reviews.

## The loop for one task

1. Pick the next task in the current phase spec whose dependencies are merged.
2. Implementer: `/implement <task>` (Claude Code) or "Follow docs/prompts/implement.md for
   task <task>" (Codex), on a branch `task/<id>-<slug>`.
3. Read the report. Decide contract changes, proposals and open questions first.
4. Open a pull request with the template. CI must be green.
5. Reviewer: `/review <task>` in the other tool.
6. Iterate until approved, or decide yourself.
7. Review contracts line by line and the rest for behavior, then merge.

## Rules

- One task per pull request. Past roughly 600 changed lines (excluding generated files and
  snapshots), split it. Small, coupled tasks may be done in one pass.
- Specs change only by the owner. When the models disagree, fix the spec or the conventions.
- At the end of each phase, write the next phase's spec from the roadmap and update
  `architecture.md` where reality moved.

## Context hygiene

- Start each task in a fresh session; the documents carry the context.
- If something has to be explained twice, put it in `conventions.md` or an ADR.
