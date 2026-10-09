import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { api, type Task } from "@/api/client";
import { ActionButton } from "@/components/action-button";
import { Elapsed } from "@/components/elapsed";
import { Await, Fact, Page } from "@/components/page";
import { ShortId } from "@/components/short-id";
import { StatusPill } from "@/components/status-pill";
import { TranscriptView } from "@/components/transcript-view";
import { format } from "@/lib/format";
import { queries, queryKeys } from "@/lib/queries";
import { status } from "@/lib/status";
import { taskStep } from "@/lib/steps";

/** A task: its goal, state, change, and the transcript streaming as the agent works. */
export function TaskPage({ id }: { id: string }) {
  const task = useQuery(queries.task(id));
  return (
    <Await query={task} what="the task">
      {(task) => <TaskDetail task={task} />}
    </Await>
  );
}

function TaskDetail({ task }: { task: Task }) {
  const state = status.task(task);
  const queryClient = useQueryClient();
  const cancel = useMutation({
    mutationFn: () => api.cancelTask(task.id),
    onSuccess: async (updated) => {
      queryClient.setQueryData(queryKeys.task(task.id), updated);
      await queryClient.invalidateQueries({ queryKey: queryKeys.tasks(task.repo) });
    },
  });
  const ended = task.phase === "done" || task.phase === "failed" || task.phase === "cancelled";
  return (
    <Page
      title={<span className="line-clamp-3 whitespace-pre-wrap">{task.goal}</span>}
      meta={
        <>
          <StatusPill state={state} />
          {state === "running" ? (
            <>
              <span>{taskStep(task)}</span>
              <Elapsed since={task.created_at} />
            </>
          ) : null}
          <Fact label="Task">
            <ShortId id={task.id} />
          </Fact>
          {task.tool ? <Fact label="Tool">{task.tool}</Fact> : null}
          {task.sandbox ? (
            <Fact label="Sandbox">
              <Link
                to="/system"
                search={{ sandbox: task.sandbox }}
                className="font-mono text-xs hover:underline"
              >
                {format.shortId(task.sandbox)}
              </Link>
            </Fact>
          ) : null}
          <Fact label="Created">{format.time(task.created_at)}</Fact>
        </>
      }
      actions={
        <ActionButton
          label="Cancel task"
          cli={`igloo task cancel ${task.id}`}
          unmet={
            ended ? `The task already ${task.phase === "done" ? "finished" : task.phase}` : null
          }
          pending={cancel.isPending}
          onRun={() => cancel.mutate()}
        />
      }
    >
      {cancel.isError ? (
        <p role="alert" className="text-sm text-failed">
          {cancel.error.message}
        </p>
      ) : null}
      {task.error ? (
        <p
          role="alert"
          className="rounded-lg border border-errored/40 bg-errored-soft px-4 py-3 text-sm text-errored"
        >
          {task.error}
        </p>
      ) : null}
      {task.change ? <TaskChange id={task.change} /> : null}
      <Transcript task={task} />
    </Page>
  );
}

function TaskChange({ id }: { id: string }) {
  const change = useQuery(queries.change(id));
  if (!change.data) return null;
  return (
    <div className="flex items-center gap-3 rounded-lg border bg-card px-4 py-3 text-sm">
      <span className="text-muted-foreground">Change</span>
      <StatusPill state={status.change(change.data)} />
      <Link to="/changes/$id" params={{ id }} className="font-medium hover:underline">
        {change.data.title}
      </Link>
      <ShortId id={id} />
    </div>
  );
}

function Transcript({ task }: { task: Task }) {
  const transcript = useQuery(queries.transcript(task.id));
  const [follow, setFollow] = useState(true);
  return (
    <section aria-label="Transcript" className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold tracking-wide text-muted-foreground uppercase">
          Transcript
        </h2>
        {transcript.data && !transcript.data.idle ? (
          <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <input
              type="checkbox"
              checked={follow}
              onChange={(event) => setFollow(event.target.checked)}
            />
            Follow
          </label>
        ) : null}
      </div>
      <Await query={transcript} what="the transcript">
        {(data) => (
          <TranscriptView
            transcript={data}
            turns={task.turns}
            follow={follow && !data.idle}
            onLeaveEnd={() => setFollow(false)}
          />
        )}
      </Await>
    </section>
  );
}
