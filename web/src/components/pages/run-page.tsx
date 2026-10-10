import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { FileText } from "lucide-react";
import type { Run } from "@/api/client";
import { ChecksStrip } from "@/components/checks-strip";
import { CheckTime, Timing } from "@/components/elapsed";
import { Await, Fact, Page, Section } from "@/components/page";
import { RelativeTime } from "@/components/relative-time";
import { ShortId } from "@/components/short-id";
import { StatusPill } from "@/components/status-pill";
import { format } from "@/lib/format";
import { queries } from "@/lib/queries";
import { status } from "@/lib/status";
import { previousDuration, runStep } from "@/lib/steps";
import { timing } from "@/lib/timing";

/** A run: the checks of one revision, with a link to each check's log. */
export function RunPage({ id }: { id: string }) {
  const run = useQuery(queries.run(id));
  return (
    <Await query={run} what="the run">
      {(run) => {
        const state = status.run(run);
        return (
          <Page
            crumbs={[
              { label: "Changes", link: { to: "/changes" } },
              {
                label: <span className="font-mono text-sm">{format.shortId(run.change)}</span>,
                link: { to: "/changes/$id", params: { id: run.change } },
              },
              { label: <span className="font-mono text-sm">{format.shortId(run.id)}</span> },
            ]}
            title={`Run of revision ${run.revision}`}
            meta={
              <>
                <StatusPill state={state} />
                {state === "running" ? (
                  <>
                    <span>{runStep(run)}</span>
                    <Timing {...timing.run(run)} />
                  </>
                ) : null}
                <Fact label="Run">
                  <ShortId id={run.id} />
                </Fact>
                <Fact label="Change">
                  <Link
                    to="/changes/$id"
                    params={{ id: run.change }}
                    className="font-mono text-sm hover:underline"
                  >
                    {format.shortId(run.change)}
                  </Link>
                </Fact>
                <Fact label="Revision">
                  <span className="tabular">{run.revision}</span>
                </Fact>
                <Fact label="Commit">
                  <ShortId id={run.commit} />
                </Fact>
                {run.sandbox ? (
                  <Fact label="Sandbox">
                    <Link
                      to="/system"
                      search={{ sandbox: run.sandbox }}
                      className="font-mono text-sm hover:underline"
                    >
                      {format.shortId(run.sandbox)}
                    </Link>
                  </Fact>
                ) : null}
                <Fact label="Started">
                  <RelativeTime at={run.started_at} />
                </Fact>
                {run.ended_at ? (
                  <Fact label="Ended">
                    <RelativeTime at={run.ended_at} />
                  </Fact>
                ) : null}
              </>
            }
          >
            {run.error ? (
              <p
                role="alert"
                className="rounded-lg border border-errored/30 bg-errored-soft px-4 py-3 text-base text-errored"
              >
                {run.error}
              </p>
            ) : null}
            {run.warm_job ? (
              <Link
                to="/jobs/$id"
                params={{ id: run.warm_job }}
                className="flex items-center gap-2 text-base text-muted-foreground hover:text-foreground"
              >
                <FileText className="size-4" />
                Warm-up log
              </Link>
            ) : null}
            <Section
              title="Checks"
              count={run.checks.length}
              action={<ChecksStrip checks={run.checks} />}
            >
              <ul aria-label="Checks" className="divide-y divide-border border-y">
                {run.checks.map((check) => (
                  <li key={check.name} className="flex items-center gap-3 px-4 py-2.5 text-base">
                    <StatusPill state={status.check(check)} className="w-24 justify-start" />
                    <span className="font-medium">{check.name}</span>
                    {check.reason ? (
                      <span className="min-w-0 truncate text-errored">{check.reason}</span>
                    ) : null}
                    <div className="ml-auto flex items-center gap-2">
                      <CheckTime check={check} />
                      <PreviousDuration run={run} name={check.name} />
                      {check.job ? (
                        <Link
                          to="/jobs/$id"
                          params={{ id: check.job }}
                          className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-sm text-muted-foreground hover:bg-accent hover:text-foreground"
                        >
                          Log
                        </Link>
                      ) : null}
                    </div>
                  </li>
                ))}
              </ul>
            </Section>
          </Page>
        );
      }}
    </Await>
  );
}

/** "previously 3m 05s": how long check `name` took in the change's previous run that ran it. */
function PreviousDuration({ run, name }: { run: Run; name: string }) {
  const runs = useQuery(queries.changeRuns(run.change));
  const previous = runs.data ? previousDuration(runs.data, run, name) : null;
  return previous === null ? null : (
    <span className="text-sm text-muted-foreground">previously {format.duration(previous)}</span>
  );
}
