import { useQuery } from "@tanstack/react-query";
import { LogView } from "@/components/log-view";
import { Await, Fact, Page } from "@/components/page";
import { ShortId } from "@/components/short-id";
import { StatusPill } from "@/components/status-pill";
import { queries } from "@/lib/queries";
import { status } from "@/lib/status";

/** A job's full log, following while it runs. */
export function JobPage({ id }: { id: string }) {
  const job = useQuery(queries.job(id));
  return (
    <Page
      title="Job log"
      meta={
        <>
          {job.data ? (
            <StatusPill
              state={status.job(job.data)}
              label={job.data.phase === "finished" ? `Exited ${job.data.exit_code}` : undefined}
            />
          ) : null}
          <Fact label="Job">
            <ShortId id={id} />
          </Fact>
          {job.data?.argv.length ? (
            <code className="truncate text-xs">{job.data.argv.join(" ")}</code>
          ) : null}
        </>
      }
    >
      {job.data?.failure_reason ? (
        <p
          role="alert"
          className="rounded-lg border border-errored/40 bg-errored-soft px-4 py-3 text-sm text-errored"
        >
          {job.data.failure_reason}
        </p>
      ) : null}
      {job.isError ? (
        <Await query={job} what="the job">
          {() => null}
        </Await>
      ) : null}
      <LogView jobId={id} className="h-[70vh]" />
    </Page>
  );
}
