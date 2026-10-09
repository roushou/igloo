import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Elapsed } from "@/components/elapsed";
import { Await, Fact, Page, Rows } from "@/components/page";
import { ShortId } from "@/components/short-id";
import { StatusPill } from "@/components/status-pill";
import { format } from "@/lib/format";
import { queries } from "@/lib/queries";
import { status } from "@/lib/status";
import { runStep } from "@/lib/steps";

/** A run: the checks of one revision, with a link to each check's log. */
export function RunPage({ id }: { id: string }) {
  const run = useQuery(queries.run(id));
  return (
    <Await query={run} what="the run">
      {(run) => {
        const state = status.run(run);
        return (
          <Page
            title="Run"
            meta={
              <>
                <StatusPill state={state} />
                {state === "running" ? (
                  <>
                    <span>{runStep(run)}</span>
                    <Elapsed since={run.started_at} />
                  </>
                ) : null}
                <Fact label="Run">
                  <ShortId id={run.id} />
                </Fact>
                <Fact label="Change">
                  <Link
                    to="/changes/$id"
                    params={{ id: run.change }}
                    className="font-mono text-xs hover:underline"
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
                <Fact label="Started">{format.time(run.started_at)}</Fact>
              </>
            }
          >
            {run.error ? (
              <p
                role="alert"
                className="rounded-lg border border-errored/40 bg-errored-soft px-4 py-3 text-sm text-errored"
              >
                {run.error}
              </p>
            ) : null}
            {run.warm_job ? (
              <p className="text-sm">
                <Link to="/jobs/$id" params={{ id: run.warm_job }} className="hover:underline">
                  Warm-up log
                </Link>
              </p>
            ) : null}
            <Rows label="Checks">
              {run.checks.map((check) => (
                <li key={check.name} className="flex items-center gap-3 px-4 py-3 text-sm">
                  <StatusPill state={status.check(check)} className="w-24" />
                  <span className="font-medium">{check.name}</span>
                  {check.reason ? <span className="text-errored">{check.reason}</span> : null}
                  {check.job ? (
                    <Link
                      to="/jobs/$id"
                      params={{ id: check.job }}
                      className="ml-auto text-xs hover:underline"
                    >
                      Log
                    </Link>
                  ) : null}
                </li>
              ))}
            </Rows>
          </Page>
        );
      }}
    </Await>
  );
}
