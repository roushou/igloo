import type { components } from "@/api/schema.gen";

type S = components["schemas"];

/** The six states every page shows, in the order a reader should attend to them. */
export type State = "needs-you" | "running" | "passed" | "failed" | "errored" | "closed";

/** What each state is called on screen. */
export const STATE_LABELS: Record<State, string> = {
  "needs-you": "Needs you",
  running: "Running",
  passed: "Passed",
  failed: "Failed",
  errored: "Errored",
  closed: "Closed",
};

/** What a workspace's pill says: its phase. */
export const WORKSPACE_LABELS: Record<S["WorkspacePhase"], string> = {
  starting: "Starting",
  running: "Running",
  stopping: "Stopping",
  stopped: "Stopped",
};

/** Every state, most urgent first. */
export const STATES: readonly State[] = [
  "needs-you",
  "running",
  "failed",
  "errored",
  "passed",
  "closed",
];

/**
 * Maps every phase enum of the API to one of the six states. This module is the only place the
 * mapping lives; pages ask it and never switch on a phase themselves.
 */
export const status = {
  task(task: Pick<S["TaskResource"], "phase">): State {
    switch (task.phase) {
      case "awaiting_review":
        return "needs-you";
      case "preparing":
      case "working":
        return "running";
      case "done":
        return "passed";
      case "failed":
        return "failed";
      case "cancelled":
        return "closed";
    }
  },

  /**
   * A change that is open waits on the user when its checks passed or it needs approval; while
   * its checks run, or are yet to start, it runs; a failed check fails it.
   */
  change(change: Pick<S["ChangeResource"], "phase" | "readiness">): State {
    switch (change.phase) {
      case "merged":
        return "passed";
      case "closed":
        return "closed";
      case "open":
        break;
    }
    const readiness = change.readiness;
    if (!readiness) return "running";
    if (readiness.checks === "passed" || readiness.approval.state === "required") {
      return "needs-you";
    }
    return readiness.checks === "failed" ? "failed" : "running";
  },

  run(run: Pick<S["RunResource"], "phase">): State {
    switch (run.phase) {
      case "preparing":
      case "warming":
      case "checking":
        return "running";
      case "passed":
        return "passed";
      case "failed":
        return "failed";
      case "errored":
        return "errored";
    }
  },

  check(check: Pick<S["CheckResource"], "status">): State {
    switch (check.status) {
      case "pending":
      case "started":
        return "running";
      case "passed":
        return "passed";
      case "failed":
        return "failed";
      case "errored":
        return "errored";
    }
  },

  /** A job that finished is passed or failed by its exit code; one that failed errored. */
  job(job: Pick<S["JobResource"], "phase" | "exit_code">): State {
    switch (job.phase) {
      case "queued":
      case "leased":
      case "running":
        return "running";
      case "finished":
        return job.exit_code === 0 ? "passed" : "failed";
      case "failed":
        return "errored";
      case "cancelled":
        return "closed";
    }
  },

  sandbox(sandbox: Pick<S["SandboxResource"], "phase">): State {
    switch (sandbox.phase) {
      case "pending":
      case "scheduled":
      case "starting":
      case "running":
      case "stopping":
        return "running";
      case "stopped":
        return "closed";
      case "failed":
        return "errored";
    }
  },

  /**
   * A workspace that is starting, running or stopping is working for the person; a stopped one is
   * closed. A failed setup does not change it: the workspace is still usable.
   */
  workspace(workspace: Pick<S["WorkspaceResource"], "phase">): State {
    switch (workspace.phase) {
      case "starting":
      case "running":
      case "stopping":
        return "running";
      case "stopped":
        return "closed";
    }
  },

  worker(worker: Pick<S["WorkerResource"], "connection">): State {
    switch (worker.connection) {
      case "connected":
        return "passed";
      case "disconnected":
        return "closed";
      case "lost":
        return "errored";
    }
  },
} as const;
