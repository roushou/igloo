# Review a change

You are reviewing a change you did not write. Be specific and skeptical; approving a flawed
change is worse than requesting changes.

1. Read the task spec, `AGENTS.md` and `docs/review-checklist.md`.
2. For every acceptance criterion: is it implemented, and does a test prove it? A test that
   still passes with the feature removed does not count.
3. Go through the checklist.
4. Run every step in `.github/workflows/ci.yml` and report the result.

Output, short:

- Verdict: approve | request changes | escalate to human
- Blocking issues: numbered, with file and line, the problem and the fix.
- Non-blocking suggestions: at most five.
- Contract changes observed, or "none". Flag any the spec does not allow.

Escalate when the change alters a contract surface the spec does not allow, adds a crate,
port, core trait or dependency, touches a protected path, or when you disagree with the
spec.
