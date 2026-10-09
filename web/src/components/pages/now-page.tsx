import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { Change, Run, Task } from "@/api/client";
import { Elapsed } from "@/components/elapsed";
import { ItemRow } from "@/components/item-row";
import { Await, Empty, Page, Rows, Section } from "@/components/page";
import { WithRepo } from "@/components/repo-page";
import { ShortId } from "@/components/short-id";
import type { EventNotice } from "@/lib/event-stream";
import { useRecentActivity } from "@/lib/events";
import { format } from "@/lib/format";
import { queries } from "@/lib/queries";
import { waitingOn } from "@/lib/readiness";
import { status } from "@/lib/status";
import { runStep, taskStep } from "@/lib/steps";

/** Now: what waits on the user, what runs, and what just happened. */
export function NowPage() {
  return (
    <Page title="Now">
      <WithRepo>{(repo) => <Now repo={repo.id} />}</WithRepo>
    </Page>
  );
}

function Now({ repo }: { repo: string }) {
  const tasks = useQuery(queries.tasks(repo, ["awaiting_review", "preparing", "working"]));
  const changes = useQuery(queries.changes(repo, ["open"]));
  const runs = useQuery(queries.runs(repo));
  const activity = useRecentActivity();

  return (
    <div className="flex flex-col gap-8">
      <Await query={tasks} what="tasks">
        {(tasks) => (
          <Await query={changes} what="changes">
            {(changes) => <Waiting tasks={tasks} changes={changes} />}
          </Await>
        )}
      </Await>
      <Await query={tasks} what="tasks">
        {(tasks) => (
          <Await query={runs} what="runs">
            {(runs) => <Running tasks={tasks} runs={runs} />}
          </Await>
        )}
      </Await>
      <Await query={runs} what="runs">
        {(runs) => <RecentRuns runs={runs} />}
      </Await>
      <Activity notices={activity} />
    </div>
  );
}

function Waiting({ tasks, changes }: { tasks: Task[]; changes: Change[] }) {
  const waitingTasks = tasks.filter((task) => status.task(task) === "needs-you");
  const waitingChanges = changes.filter((change) => status.change(change) === "needs-you");
  const count = waitingTasks.length + waitingChanges.length;
  return (
    <Section title="Needs you" count={count} tone="needs-you">
      {count === 0 ? (
        <Empty>Nothing is waiting on you.</Empty>
      ) : (
        <Rows label="Needs you">
          {waitingChanges.map((change) => (
            <ItemRow
              key={change.id}
              state="needs-you"
              title={change.title}
              link={{ to: "/changes/$id", params: { id: change.id } }}
              details={
                <>
                  <span>{waitingOn(change)}</span>
                  <ShortId id={change.id} />
                </>
              }
            />
          ))}
          {waitingTasks.map((task) => (
            <ItemRow
              key={task.id}
              state="needs-you"
              title={task.goal}
              link={{ to: "/tasks/$id", params: { id: task.id } }}
              details={
                <>
                  <span>The agent finished and awaits your review</span>
                  <ShortId id={task.id} />
                </>
              }
            />
          ))}
        </Rows>
      )}
    </Section>
  );
}

function Running({ tasks, runs }: { tasks: Task[]; runs: Run[] }) {
  const runningTasks = tasks.filter((task) => status.task(task) === "running");
  const runningRuns = runs.filter((run) => status.run(run) === "running");
  const count = runningTasks.length + runningRuns.length;
  return (
    <Section title="Running" count={count}>
      {count === 0 ? (
        <Empty>Nothing is running.</Empty>
      ) : (
        <Rows label="Running">
          {runningTasks.map((task) => (
            <ItemRow
              key={task.id}
              state="running"
              title={task.goal}
              link={{ to: "/tasks/$id", params: { id: task.id } }}
              details={
                <>
                  <span>{taskStep(task)}</span>
                  <ShortId id={task.id} />
                </>
              }
              aside={<Elapsed since={task.created_at} />}
            />
          ))}
          {runningRuns.map((run) => (
            <ItemRow
              key={run.id}
              state="running"
              title={`Checks of ${format.shortId(run.change)}, revision ${run.revision}`}
              link={{ to: "/runs/$id", params: { id: run.id } }}
              details={
                <>
                  <span>{runStep(run)}</span>
                  <ShortId id={run.id} />
                </>
              }
              aside={<Elapsed since={run.started_at} />}
            />
          ))}
        </Rows>
      )}
    </Section>
  );
}

const RECENT_RUNS = 8;

function RecentRuns({ runs }: { runs: Run[] }) {
  const finished = runs.filter((run) => status.run(run) !== "running").slice(0, RECENT_RUNS);
  return (
    <Section title="Recent runs" count={finished.length}>
      {finished.length === 0 ? (
        <Empty>No run has finished yet.</Empty>
      ) : (
        <Rows label="Recent runs">
          {finished.map((run) => (
            <ItemRow
              key={run.id}
              state={status.run(run)}
              title={`Checks of ${format.shortId(run.change)}, revision ${run.revision}`}
              link={{ to: "/runs/$id", params: { id: run.id } }}
              details={
                <>
                  {run.error ? <span className="text-errored">{run.error}</span> : null}
                  <ShortId id={run.id} />
                </>
              }
              aside={format.time(run.started_at)}
            />
          ))}
        </Rows>
      )}
    </Section>
  );
}

const ACTIVITY_SHOWN = 15;

function Activity({ notices }: { notices: readonly EventNotice[] }) {
  return (
    <Section title="Recent activity">
      {notices.length === 0 ? (
        <Empty>Events appear here as they happen.</Empty>
      ) : (
        <ul aria-label="Recent activity" className="flex flex-col gap-1 text-sm">
          {notices.slice(0, ACTIVITY_SHOWN).map((notice) => (
            <li key={notice.sequence} className="flex items-baseline gap-3">
              <time className="tabular w-20 shrink-0 text-xs text-muted-foreground">
                {new Date(notice.time).toLocaleTimeString()}
              </time>
              <span>{notice.kind.replaceAll("_", " ").replaceAll(".", " ")}</span>
              <Subject notice={notice} />
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

function Subject({ notice }: { notice: EventNotice }) {
  const id = notice.resource_id;
  switch (notice.resource_type) {
    case "task":
      return (
        <Link to="/tasks/$id" params={{ id }} className="font-mono text-xs hover:underline">
          {format.shortId(id)}
        </Link>
      );
    case "change":
      return (
        <Link to="/changes/$id" params={{ id }} className="font-mono text-xs hover:underline">
          {format.shortId(id)}
        </Link>
      );
    case "run":
      return (
        <Link to="/runs/$id" params={{ id }} className="font-mono text-xs hover:underline">
          {format.shortId(id)}
        </Link>
      );
    case "job":
      return (
        <Link to="/jobs/$id" params={{ id }} className="font-mono text-xs hover:underline">
          {format.shortId(id)}
        </Link>
      );
    default:
      return <ShortId id={id} />;
  }
}
