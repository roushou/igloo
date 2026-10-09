import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { api, type Task } from "@/api/client";
import { CliCommand } from "@/components/action-button";
import { ItemRow } from "@/components/item-row";
import { Await, Empty, Page, Rows, Section } from "@/components/page";
import { WithRepo } from "@/components/repo-page";
import { ShortId } from "@/components/short-id";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { format } from "@/lib/format";
import { queries, queryKeys } from "@/lib/queries";
import { STATE_LABELS, STATES, status } from "@/lib/status";
import { taskStep } from "@/lib/steps";

/** Tasks: every task of the repository, grouped by state, and the form that creates one. */
export function TasksPage() {
  return (
    <Page title="Tasks">
      <WithRepo>{(repo) => <Tasks repo={repo.id} />}</WithRepo>
    </Page>
  );
}

function Tasks({ repo }: { repo: string }) {
  const tasks = useQuery(queries.tasks(repo));
  return (
    <div className="flex flex-col gap-8">
      <CreateTask repo={repo} />
      <Await query={tasks} what="tasks">
        {(tasks) =>
          tasks.length === 0 ? (
            <Empty>No task yet. Create one to put an agent to work.</Empty>
          ) : (
            STATES.map((state) => {
              const group = tasks.filter((task) => status.task(task) === state);
              return group.length === 0 ? null : (
                <Section
                  key={state}
                  title={STATE_LABELS[state]}
                  count={group.length}
                  tone={state === "needs-you" ? "needs-you" : undefined}
                >
                  <Rows label={STATE_LABELS[state]}>
                    {group.map((task) => (
                      <TaskRow key={task.id} task={task} />
                    ))}
                  </Rows>
                </Section>
              );
            })
          )
        }
      </Await>
    </div>
  );
}

function TaskRow({ task }: { task: Task }) {
  const state = status.task(task);
  return (
    <ItemRow
      state={state}
      title={task.goal}
      link={{ to: "/tasks/$id", params: { id: task.id } }}
      details={
        <>
          {state === "running" ? <span>{taskStep(task)}</span> : null}
          {task.error ? <span className="text-errored">{task.error}</span> : null}
          <ShortId id={task.id} />
          {task.tool ? <span>{task.tool}</span> : null}
        </>
      }
      aside={format.time(task.created_at)}
    />
  );
}

function CreateTask({ repo }: { repo: string }) {
  const [open, setOpen] = useState(false);
  const [goal, setGoal] = useState("");
  const [tool, setTool] = useState("");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const create = useMutation({
    mutationFn: () => api.createTask(repo, goal.trim(), tool.trim() || undefined),
    onSuccess: async (task) => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.tasks(repo) });
      setOpen(false);
      setGoal("");
      setTool("");
      await navigate({ to: "/tasks/$id", params: { id: task.id } });
    },
  });
  if (!open) {
    return (
      <div>
        <Button onClick={() => setOpen(true)}>Create task</Button>
      </div>
    );
  }
  const cli = `igloo task create ${JSON.stringify(goal.trim() || "<goal>")}${tool.trim() ? ` --tool ${tool.trim()}` : ""} --repo ${repo}`;
  return (
    <form
      aria-label="Create task"
      className="flex max-w-2xl flex-col gap-3 rounded-lg border bg-card p-4"
      onSubmit={(event) => {
        event.preventDefault();
        create.mutate();
      }}
    >
      <label className="flex flex-col gap-1 text-sm font-medium">
        Goal
        <textarea
          required
          rows={4}
          className="rounded-md border bg-transparent px-3 py-2 text-sm font-normal outline-none focus-visible:ring-2 focus-visible:ring-ring"
          placeholder="What should the agent do?"
          value={goal}
          onChange={(event) => setGoal(event.target.value)}
        />
      </label>
      <div className="flex flex-col gap-1 text-sm font-medium">
        <label htmlFor="task-tool">Tool</label>
        <Input
          id="task-tool"
          placeholder="The repository's default tool"
          className="font-normal"
          value={tool}
          onChange={(event) => setTool(event.target.value)}
        />
      </div>
      {create.isError ? (
        <p role="alert" className="text-sm text-failed">
          {create.error.message}
        </p>
      ) : null}
      <CliCommand command={cli} />
      <div className="flex gap-2">
        <Button type="submit" disabled={!goal.trim() || create.isPending}>
          {create.isPending ? "Creating…" : "Create"}
        </Button>
        <Button type="button" variant="ghost" onClick={() => setOpen(false)}>
          Cancel
        </Button>
      </div>
    </form>
  );
}
