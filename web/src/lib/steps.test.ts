import { checkDuration, previousDuration } from "./steps";

const check = (name: string, started_at?: string, ended_at?: string) => ({
  name,
  status: "passed" as const,
  started_at,
  ended_at,
});

describe("checkDuration", () => {
  it("is the time between a check's start and end", () => {
    expect(checkDuration(check("test", "2026-01-01T00:00:00Z", "2026-01-01T00:01:30Z"))).toBe(
      90_000,
    );
  });

  it("is unknown until the check ended", () => {
    expect(checkDuration(check("test", "2026-01-01T00:00:00Z"))).toBeNull();
    expect(checkDuration(check("test"))).toBeNull();
  });
});

describe("previousDuration", () => {
  const earlier = (n: number, started_at: string, duration?: string) => ({
    id: `run_${n}`,
    started_at,
    checks: [check("test", "2026-01-01T00:00:00Z", duration)],
  });
  const current = { id: "run_9", started_at: "2026-01-03T00:00:00Z" };

  it("takes the latest earlier run that ran the check to the end", () => {
    const runs = [
      earlier(1, "2026-01-01T00:00:00Z", "2026-01-01T00:00:10Z"),
      earlier(2, "2026-01-02T00:00:00Z", "2026-01-01T00:00:20Z"),
      earlier(3, "2026-01-02T12:00:00Z"),
      { ...current, checks: [] },
    ];
    expect(previousDuration(runs, current, "test")).toBe(20_000);
  });

  it("ignores later runs and unknown checks", () => {
    const runs = [earlier(4, "2026-01-04T00:00:00Z", "2026-01-01T00:00:10Z")];
    expect(previousDuration(runs, current, "test")).toBeNull();
    expect(
      previousDuration(runs, { ...current, started_at: "2026-01-05T00:00:00Z" }, "lint"),
    ).toBeNull();
  });
});
