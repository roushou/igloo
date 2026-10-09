import { change, readiness } from "@/test/fixtures";
import { type State, status } from "./status";

describe("status", () => {
  it.each<[string, string, State]>([
    ["task", "awaiting_review", "needs-you"],
    ["task", "preparing", "running"],
    ["task", "working", "running"],
    ["task", "done", "passed"],
    ["task", "failed", "failed"],
    ["task", "cancelled", "closed"],
    ["run", "preparing", "running"],
    ["run", "warming", "running"],
    ["run", "checking", "running"],
    ["run", "passed", "passed"],
    ["run", "failed", "failed"],
    ["run", "errored", "errored"],
    ["job", "queued", "running"],
    ["job", "leased", "running"],
    ["job", "running", "running"],
    ["job", "failed", "errored"],
    ["job", "cancelled", "closed"],
    ["sandbox", "running", "running"],
    ["sandbox", "failed", "errored"],
    ["sandbox", "stopped", "closed"],
  ])("maps %s %s to %s", (kind, phase, expected) => {
    const resource = { phase } as never;
    expect(status[kind as "task" | "run" | "job" | "sandbox"](resource)).toBe(expected);
  });

  it("tells a finished job's exit code apart", () => {
    expect(status.job({ phase: "finished", exit_code: 0 })).toBe("passed");
    expect(status.job({ phase: "finished", exit_code: 2 })).toBe("failed");
  });

  it("maps changes by phase and readiness", () => {
    expect(status.change(change(1, { phase: "merged" }))).toBe("passed");
    expect(status.change(change(1, { phase: "closed" }))).toBe("closed");
    expect(status.change(change(1))).toBe("needs-you");
    expect(status.change(change(1, { readiness: readiness({ checks: "running" }) }))).toBe(
      "running",
    );
    expect(status.change(change(1, { readiness: readiness({ checks: "failed" }) }))).toBe("failed");
    expect(
      status.change(
        change(1, {
          readiness: readiness({
            checks: "running",
            approval: { state: "required", protected_paths: ["proto/a.proto"] },
          }),
        }),
      ),
    ).toBe("needs-you");
  });

  it("maps check statuses", () => {
    expect(status.check({ status: "pending" })).toBe("running");
    expect(status.check({ status: "errored" })).toBe("errored");
  });
});
