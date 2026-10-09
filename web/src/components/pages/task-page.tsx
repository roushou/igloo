import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Ban, GitPullRequest } from "lucide-react";
import { useMemo, useState } from "react";
import { api, type Task } from "@/api/client";
import { ActionButton, CliMenu } from "@/components/action-button";
import { Timing } from "@/components/elapsed";
import { Await, Fact, Page } from "@/components/page";
import { RelativeTime } from "@/components/relative-time";
import { ShortId } from "@/components/short-id";
import { StatusPill } from "@/components/status-pill";
import { TranscriptView } from "@/components/transcript-view";
import { useToast } from "@/components/ui/toast";
import { type Command, useRegisterCommands } from "@/lib/commands";
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
  const toast = useToast();
  const cancel = useMutation({
    mutationFn: () => api.cancelTask(task.id),
    onSuccess: async (updated) => {
      queryClient.setQueryData(queryKeys.task(task.id), updated);
      await queryClient.invalidateQueries({ queryKey: queryKeys.tasks(task.repo) });
      toast.show({ title: "Task cancelled", description: "Its sandbox is stopping." });
    },
    onError: (error) =>
      toast.show({ title: "Could not cancel the task", description: error.message, tone: "error" }),
  });
  const ended = task.phase === "done" || task.phase === "failed" || task.phase === "cancelled";
  const unmet = ended
    ? `The task already ${task.phase === "done" ? "finished" : task.phase}`
    : null;

  const commands = useMemo<Command[]>(
    () => [
      {
        id: "cancel-task",
        label: "Cancel this task",
        group: "This task",
        icon: Ban,
        keywords: ["stop"],
        unmet,
        run: () => cancel.mutate(),
      },
    ],
    [unmet, cancel.mutate],
  );
  useRegisterCommands(commands);

  return (
    <Page
      crumbs={[
        { label: "Tasks", link: { to: "/tasks", search: true } },
        { label: <span className="font-mono text-sm">{format.shortId(task.id)}</span> },
      ]}
      compactTitle
      title={<span className="line-clamp-3 whitespace-pre-wrap">{task.goal}</span>}
      meta={
        <>
          <StatusPill state={state} />
          {state === "running" ? (
            <>
              <span>{taskStep(task)}</span>
              <Timing startedAt={task.created_at} />
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
                className="font-mono text-sm hover:underline"
              >
                {format.shortId(task.sandbox)}
              </Link>
            </Fact>
          ) : null}
          <Fact label="Created">
            <RelativeTime at={task.created_at} />
          </Fact>
        </>
      }
      actions={
        <>
          <CliMenu commands={[{ label: "Cancel task", command: `igloo task cancel ${task.id}` }]} />
          <ActionButton
            label="Cancel task"
            unmet={unmet}
            pending={cancel.isPending}
            onRun={() => cancel.mutate()}
          />
        </>
      }
    >
      {task.error ? (
        <p
          role="alert"
          className="rounded-lg border border-errored/30 bg-errored-soft px-4 py-3 text-base text-errored"
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
    <Link
      to="/changes/$id"
      params={{ id }}
      className="flex items-center gap-3 rounded-lg border px-4 py-3 text-base hover:bg-accent/60"
    >
      <GitPullRequest className="size-4 text-muted-foreground" />
      <span className="text-muted-foreground">Change</span>
      <StatusPill state={status.change(change.data)} />
      <span className="min-w-0 flex-1 truncate font-medium">{change.data.title}</span>
      <span className="hidden font-mono text-sm text-muted-foreground sm:inline">
        {format.shortId(id)}
      </span>
    </Link>
  );
}

function Transcript({ task }: { task: Task }) {
  const transcript = useQuery(queries.transcript(task.id));
  const [follow, setFollow] = useState(true);
  return (
    <section aria-label="Transcript" className="flex flex-col gap-2">
      <div className="flex h-8 items-center justify-between">
        <h2 className="font-sans text-base font-semibold">Transcript</h2>
        {transcript.data && !transcript.data.idle ? (
          <label className="flex items-center gap-2 text-sm text-muted-foreground">
            <span aria-hidden className="size-1.5 animate-pulse-dot rounded-full bg-running" />
            <input
              type="checkbox"
              className="size-3.5 accent-foreground"
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
