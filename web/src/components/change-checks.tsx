import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ArrowUpRight, ListChecks } from "lucide-react";
import { useState } from "react";
import type { Check, Run } from "@/api/client";
import { ChecksStrip } from "@/components/checks-strip";
import { CheckTime } from "@/components/elapsed";
import { EmptyState } from "@/components/empty-state";
import { LogView } from "@/components/log-view";
import { Await } from "@/components/page";
import { RelativeTime } from "@/components/relative-time";
import { StatusPill } from "@/components/status-pill";
import { queries } from "@/lib/queries";
import { status } from "@/lib/status";
import { cn } from "@/lib/utils";

/** The run that counts for `revision`: the latest one started for it. */
export function latestRun(runs: readonly Run[], revision: number): Run | null {
  return (
    runs
      .filter((run) => run.revision === revision)
      .sort((a, b) => b.started_at.localeCompare(a.started_at))[0] ?? null
  );
}

/** The check to open first: the first that failed, else the first that errored, else the first. */
function firstToOpen(checks: readonly Check[]): Check | null {
  return (
    checks.find((check) => check.status === "failed") ??
    checks.find((check) => check.status === "errored") ??
    checks[0] ??
    null
  );
}

/**
 * The Checks tab: the checks of the revision's latest run, and the selected one's log. A failed
 * check is selected first and its log opens at the first line that mentions an error.
 */
export function ChecksTab({ changeId, revision }: { changeId: string; revision: number }) {
  const runs = useQuery(queries.changeRuns(changeId));
  return (
    <Await query={runs} what="the checks" rows={3}>
      {(runs) => {
        const run = latestRun(runs, revision);
        return run ? (
          <RunChecks run={run} />
        ) : (
          <EmptyState icon={ListChecks} title={`No checks have run for revision ${revision}.`}>
            Checks start when a revision is recorded; their results and logs appear here.
          </EmptyState>
        );
      }}
    </Await>
  );
}

function RunChecks({ run }: { run: Run }) {
  const [picked, setPicked] = useState<{ run: string; name: string } | null>(null);
  const selected =
    (picked?.run === run.id ? run.checks.find((check) => check.name === picked.name) : null) ??
    firstToOpen(run.checks);
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-3 text-base">
        <StatusPill state={status.run(run)} />
        <ChecksStrip checks={run.checks} />
        <span className="text-muted-foreground">
          started <RelativeTime at={run.started_at} />
        </span>
        {run.error ? <span className="text-errored">{run.error}</span> : null}
        <Link
          to="/runs/$id"
          params={{ id: run.id }}
          className="ml-auto inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
        >
          Open the run
          <ArrowUpRight className="size-3.5" />
        </Link>
      </div>
      <div className="grid gap-4 lg:grid-cols-[17rem_minmax(0,1fr)]">
        <ul aria-label="Checks" className="flex flex-col gap-0.5">
          {run.checks.map((check) => (
            <li key={check.name}>
              <button
                type="button"
                aria-pressed={check.name === selected?.name}
                className={cn(
                  "flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-base hover:bg-accent/60",
                  check.name === selected?.name && "bg-accent font-medium",
                )}
                onClick={() => setPicked({ run: run.id, name: check.name })}
              >
                <span className="min-w-0 flex-1 truncate">{check.name}</span>
                <CheckTime check={check} />
                <StatusPill state={status.check(check)} />
              </button>
            </li>
          ))}
        </ul>
        <div className="min-w-0">
          {selected ? <CheckLog key={`${run.id}-${selected.name}`} check={selected} /> : null}
        </div>
      </div>
    </div>
  );
}

function CheckLog({ check }: { check: Check }) {
  if (!check.job) {
    return (
      <p className="rounded-lg border border-dashed px-4 py-8 text-center text-base text-muted-foreground">
        {check.reason ?? "This check has not started."}
      </p>
    );
  }
  return (
    <div className="flex flex-col gap-2">
      {check.reason ? <p className="text-base text-errored">{check.reason}</p> : null}
      <LogView jobId={check.job} openAtError={check.status === "failed"} className="h-[28rem]" />
    </div>
  );
}
