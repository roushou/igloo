import type { Change, Check, Job, Run } from "@/api/client";

/** When something started and, once known, ended and last printed; what `Timing` shows. */
export type Times = {
  startedAt: string;
  endedAt?: string | null;
  lastOutputAt?: string | null;
};

/**
 * Where each thing's times come from. A function returns null while the server does not report
 * the times; every page already renders `Timing` from its result, so a field the server gains is
 * wired by reading it here.
 */
export const timing = {
  /** A run starts at `started_at`; its end is not reported yet. */
  run(run: Run): Times {
    return { startedAt: run.started_at };
  },

  /** A check's times come from its job, which the server does not time yet. */
  check(_check: Check): Times | null {
    return null;
  },

  /** A job's start, end and last output are not reported yet. */
  job(_job: Job): Times | null {
    return null;
  },

  /** When a change was merged or closed; the server does not record it yet. */
  changeEnd(_change: Change): string | null {
    return null;
  },
};
