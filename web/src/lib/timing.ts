import type { Change, Check, Job, Run } from "@/api/client";

/** When something started and, once known, ended and last printed; what `Timing` shows. */
export type Times = {
  startedAt: string;
  endedAt?: string | null;
  lastOutputAt?: string | null;
};

/**
 * Where each thing's times come from, as the server reports them. A function returns null while
 * a thing has not started; every page renders `Timing` from its result.
 */
export const timing = {
  /** A run starts at `started_at` and ends once it has an outcome. */
  run(run: Run): Times {
    return { startedAt: run.started_at, endedAt: run.ended_at };
  },

  /** A check's times are its job's; null until its job starts. */
  check(check: Check): Times | null {
    return check.started_at ? { startedAt: check.started_at, endedAt: check.ended_at } : null;
  },

  /** A job's start and end; null until it starts. Its last output is measured in the browser. */
  job(job: Job): Times | null {
    return job.started_at ? { startedAt: job.started_at, endedAt: job.ended_at } : null;
  },

  /** When a change was merged or closed, if it was. */
  changeEnd(change: Change): string | null {
    return change.merged_at ?? change.closed_at ?? null;
  },
};
