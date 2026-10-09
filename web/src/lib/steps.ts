import type { Run, Task } from "@/api/client";

/** What a running task is doing now. */
export function taskStep(task: Pick<Task, "phase" | "turns">): string {
  if (task.phase === "preparing") return "Preparing the sandbox";
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
