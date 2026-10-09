import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { CheckCheck, Zap } from "lucide-react";
import type { Change, Run, Task } from "@/api/client";
import { ChecksStrip } from "@/components/checks-strip";
import { Timing } from "@/components/elapsed";
import { EmptyState } from "@/components/empty-state";
import { ItemRow } from "@/components/item-row";
import { Await, Empty, Page, Rows, Section } from "@/components/page";
import { RelativeTime } from "@/components/relative-time";
import { WithRepo } from "@/components/repo-page";
import { ShortId } from "@/components/short-id";
import { NOW_TASK_PHASES } from "@/lib/counts";
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
    <Page crumbs={[{ label: "Now" }]} title="Now">
      <WithRepo>{(repo) => <Now repo={repo.id} />}</WithRepo>
    </Page>
  );
}

function Now({ repo }: { repo: string }) {
  const tasks = useQuery(queries.tasks(repo, [...NOW_TASK_PHASES]));
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
        <div className="flex items-center gap-2 border-y py-3 text-base text-muted-foreground">
          <CheckCheck className="size-4 text-passed" />
          Nothing is waiting on you.
        </div>
      ) : (
        <Rows label="Needs you">
          {waitingChanges.map((change) => (
            <ItemRow
              key={change.id}
              state="needs-you"
              title={change.title}
              link={{ to: "/changes/$id", params: { id: change.id } }}
              details={<span className="text-expedition">{waitingOn(change)}</span>}
              meta={<ShortId id={change.id} />}
              time={<RelativeTime at={change.revisions.at(-1)?.created_at ?? ""} />}
            />
          ))}
          {waitingTasks.map((task) => (
            <ItemRow
              key={task.id}
              state="needs-you"
              title={task.goal}
              link={{ to: "/tasks/$id", params: { id: task.id } }}
              details={<span>The agent finished and awaits your review</span>}
              meta={<ShortId id={task.id} />}
              time={<RelativeTime at={task.created_at} />}
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
              details={<span>{taskStep(task)}</span>}
              meta={<ShortId id={task.id} />}
              time={<Timing startedAt={task.created_at} />}
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
                  <ChecksStrip checks={run.checks} />
                </>
              }
              meta={<ShortId id={run.id} />}
              time={<Timing startedAt={run.started_at} />}
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
        <EmptyState
          icon={Zap}
          title="No run has finished yet"
          hint="A run checks a revision of a change; it starts when a task's work or a push opens one."
        >
          Finished runs and how they ended are listed here.
        </EmptyState>
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
                  <ChecksStrip checks={run.checks} />
                </>
              }
              meta={<ShortId id={run.id} />}
              time={<RelativeTime at={run.started_at} />}
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
        <ul aria-label="Recent activity" className="flex flex-col border-l">
          {notices.slice(0, ACTIVITY_SHOWN).map((notice) => (
            <li
              key={notice.sequence}
              className="relative flex items-baseline gap-3 py-1 pl-4 text-base"
            >
              <span
                aria-hidden
                className="absolute top-3 -left-[3px] size-1.5 rounded-full bg-muted-foreground/50"
              />
              <span>{notice.kind.replaceAll("_", " ").replaceAll(".", " ")}</span>
              <Subject notice={notice} />
              <RelativeTime
                at={notice.time}
                className="tabular ml-auto text-sm text-muted-foreground"
              />
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

const SUBJECT_CLASS =
  "font-mono text-sm text-muted-foreground hover:text-foreground hover:underline";

function Subject({ notice }: { notice: EventNotice }) {
  const id = notice.resource_id;
  switch (notice.resource_type) {
    case "task":
      return (
        <Link to="/tasks/$id" params={{ id }} className={SUBJECT_CLASS}>
          {format.shortId(id)}
        </Link>
      );
    case "change":
      return (
        <Link to="/changes/$id" params={{ id }} className={SUBJECT_CLASS}>
          {format.shortId(id)}
        </Link>
      );
    case "run":
      return (
        <Link to="/runs/$id" params={{ id }} className={SUBJECT_CLASS}>
          {format.shortId(id)}
        </Link>
      );
    case "job":
      return (
        <Link to="/jobs/$id" params={{ id }} className={SUBJECT_CLASS}>
          {format.shortId(id)}
        </Link>
      );
    default:
      return <ShortId id={id} />;
  }
}
