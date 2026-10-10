import type { Check, Run, Task } from "@/api/client";

/** What a running task is doing now. */
export function taskStep(task: Pick<Task, "phase" | "turns" | "takeover">): string {
  if (task.phase === "preparing") return "Preparing the sandbox";
  switch (task.takeover?.phase) {
    case "waiting":
      return "Taken over, the current turn is finishing";
    case "paused":
      return "Taken over, paused";
    case "handing_back":
      return "Being handed back";
  }
  const turn = task.turns.at(-1);
  return turn ? `Working, turn ${turn.number}` : "Working";
}

/** What a running run is doing now; while checking, which checks run and how many are done. */
export function runStep(run: Pick<Run, "phase" | "checks">): string {
  switch (run.phase) {
    case "preparing":
      return "Preparing";
    case "warming":
      return "Warming the snapshot";
    default: {
      const running = run.checks.filter((check) => check.status === "started").map((c) => c.name);
      const done = run.checks.filter(
        (check) => check.status !== "pending" && check.status !== "started",
      ).length;
      const progress = `${done} of ${run.checks.length} checks done`;
      return running.length > 0
        ? `Checking ${running.join(", ")} (${progress})`
        : `Checking (${progress})`;
    }
  }
}

/** How long a check took, in milliseconds, once its job started and ended; `null` before. */
export function checkDuration(check: Pick<Check, "started_at" | "ended_at">): number | null {
  if (!check.started_at || !check.ended_at) return null;
  return Date.parse(check.ended_at) - Date.parse(check.started_at);
}

/**
 * How long the check `name` took in the latest run of `runs` that started before `run` and ran
 * it to the end; `null` when no earlier run did.
 */
export function previousDuration(
  runs: readonly Pick<Run, "id" | "started_at" | "checks">[],
  run: Pick<Run, "id" | "started_at">,
  name: string,
): number | null {
  const earlier = runs
    .filter((other) => other.id !== run.id && other.started_at < run.started_at)
    .sort((a, b) => b.started_at.localeCompare(a.started_at));
  for (const other of earlier) {
    const check = other.checks.find((check) => check.name === name);
    const duration = check ? checkDuration(check) : null;
    if (duration !== null) return duration;
  }
  return null;
}
