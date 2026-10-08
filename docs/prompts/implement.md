# Implement a task

You are implementing one task from `docs/specs/`. The task id is given with this prompt.

1. Read `AGENTS.md`, `docs/conventions.md` and the task's "Read first" items.
2. Restate the goal, acceptance criteria and allowed contract changes in a few lines. If
   anything is ambiguous, stop and ask with concrete options.
3. Write tests for the acceptance criteria first and confirm they fail for the right reason.
4. Implement the smallest change that makes them pass, following the conventions.
5. Run clippy and nextest after each step, and every CI step at the end. Fix failures
   properly; never weaken a test, lint or check.
6. Re-read your diff against `docs/review-checklist.md`.
7. Finish with the short report from `AGENTS.md`.

Stay inside the task's scope. New dependencies, crates, ports or core traits are proposals
in the report, not silent additions.
