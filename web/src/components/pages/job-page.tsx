import { useQuery } from "@tanstack/react-query";
import { Timing } from "@/components/elapsed";
import { LogView } from "@/components/log-view";
import { Await, Fact, Page } from "@/components/page";
import { ShortId } from "@/components/short-id";
import { StatusPill } from "@/components/status-pill";
import { format } from "@/lib/format";
import { queries } from "@/lib/queries";
import { status } from "@/lib/status";
import { timing } from "@/lib/timing";

/** A job's full log, following while it runs. */
export function JobPage({ id }: { id: string }) {
  const job = useQuery(queries.job(id));
  const times = job.data ? timing.job(job.data) : null;
  return (
    <Page
      wide
      crumbs={[
        { label: "Jobs" },
        { label: <span className="font-mono text-sm">{format.shortId(id)}</span> },
      ]}
      title="Job log"
      meta={
        <>
          {job.data ? (
            <StatusPill
              state={status.job(job.data)}
              label={job.data.phase === "finished" ? `Exited ${job.data.exit_code}` : undefined}
            />
          ) : null}
          {times ? <Timing {...times} /> : null}
          <Fact label="Job">
            <ShortId id={id} />
          </Fact>
          {job.data?.started_at ? (
            <Fact label="Started">{format.time(job.data.started_at)}</Fact>
          ) : null}
          {job.data?.started_at && job.data.ended_at ? (
            <Fact label="Took">
              {format.duration(Date.parse(job.data.ended_at) - Date.parse(job.data.started_at))}
            </Fact>
          ) : null}
          {job.data?.argv.length ? (
            <code className="max-w-full truncate rounded bg-muted px-1.5 py-0.5 text-sm text-foreground">
              {job.data.argv.join(" ")}
            </code>
          ) : null}
        </>
      }
    >
      {job.data?.failure_reason ? (
        <p
          role="alert"
          className="rounded-lg border border-errored/30 bg-errored-soft px-4 py-3 text-base text-errored"
        >
          {job.data.failure_reason}
        </p>
      ) : null}
      {job.isError ? (
        <Await query={job} what="the job">
          {() => null}
        </Await>
      ) : null}
      <LogView jobId={id} className="h-[calc(100dvh-16rem)] min-h-80" />
    </Page>
  );
}
