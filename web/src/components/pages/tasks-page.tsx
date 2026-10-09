import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getRouteApi, Outlet, useMatch, useNavigate } from "@tanstack/react-router";
import { ListChecks, Plus } from "lucide-react";
import { useState } from "react";
import { api, type Task } from "@/api/client";
import { CliCommand, CliMenu } from "@/components/action-button";
import { Timing } from "@/components/elapsed";
import { EmptyState } from "@/components/empty-state";
import { FilterBar } from "@/components/filter-bar";
import { ItemRow } from "@/components/item-row";
import { ListPane } from "@/components/list-pane";
import { Await, Empty, Loading, Rows, Section } from "@/components/page";
import { RelativeTime } from "@/components/relative-time";
import { WithRepo } from "@/components/repo-page";
import { ShortId } from "@/components/short-id";
import { Button } from "@/components/ui/button";
import { Modal } from "@/components/ui/dialog";
import { Input, Textarea } from "@/components/ui/input";
import { Kbd } from "@/components/ui/kbd";
import { useToast } from "@/components/ui/toast";
import { matchesQuery } from "@/lib/list-search";
import { useIsWide } from "@/lib/media";
import { queries, queryKeys } from "@/lib/queries";
import { STATE_LABELS, STATES, type State, status } from "@/lib/status";
import { taskStep } from "@/lib/steps";

const route = getRouteApi("/_app/tasks");

/**
 * Tasks: every task of the repository, grouped by state and filtered from the URL. A task opens
 * beside the list on a wide viewport, and in its place on a narrow one.
 */
export function TasksLayout() {
  const open = useMatch({ from: "/_app/tasks/$id", shouldThrow: false });
  const wide = useIsWide();
  const detail = Boolean(open);
  return (
    <div className="flex h-full min-h-0">
      {detail && !wide ? null : (
        <WithRepo
          placeholder={
            <ListPane title="Tasks" narrow={detail}>
              <Loading what="tasks" />
            </ListPane>
          }
        >
          {(repo) => <TaskList repo={repo.id} narrow={detail} active={open?.params.id} />}
        </WithRepo>
      )}
      {detail ? (
        <div className="min-w-0 flex-1">
          <Outlet />
        </div>
      ) : null}
    </div>
  );
}

function TaskList({ repo, narrow, active }: { repo: string; narrow: boolean; active?: string }) {
  const search = route.useSearch();
  const navigate = useNavigate();
  const tasks = useQuery(queries.tasks(repo));
  const data = tasks.data ?? [];
  const counts: Partial<Record<State, number>> = {};
  for (const task of data) counts[status.task(task)] = (counts[status.task(task)] ?? 0) + 1;

  const setSearch = (patch: { state?: State | undefined; q?: string }, replace = false) =>
    void navigate({ to: "/tasks", search: (prev) => ({ ...prev, ...patch }), replace });

  return (
    <ListPane
      title="Tasks"
      narrow={narrow}
      actions={
        <>
          <CliMenu
            commands={[
              { label: "Create a task", command: `igloo task create "<goal>" --repo ${repo}` },
            ]}
          />
          <Button
            size={narrow ? "icon" : "default"}
            aria-label="Create task"
            onClick={() => setSearch({ new: true } as never)}
          >
            <Plus />
            {narrow ? null : (
              <>
                Create task{" "}
                <Kbd className="border-primary-foreground/30 bg-transparent text-primary-foreground/70">
                  C
                </Kbd>
              </>
            )}
          </Button>
        </>
      }
      toolbar={
        data.length > 0 ? (
          <FilterBar
            what="tasks"
            counts={counts}
            state={search.state}
            onState={(state) => setSearch({ state })}
            query={search.q ?? ""}
            onQuery={(q) => setSearch({ q: q || undefined }, true)}
          />
        ) : null
      }
    >
      <Await query={tasks} what="tasks">
        {(tasks) =>
          tasks.length === 0 ? (
            <EmptyState
              icon={ListChecks}
              title="No tasks yet"
              command={`igloo task create "Fix the flaky checkout test" --repo ${repo}`}
              hint={
                <>
                  Or ask your agent: the <code className="font-mono">task.create</code> MCP tool
                  does the same.
                </>
              }
            >
              A task puts an agent to work on a goal. Its transcript, and the change it opens,
              appear here.
            </EmptyState>
          ) : (
            <Groups tasks={tasks} state={search.state} query={search.q} active={active} />
          )
        }
      </Await>
      <CreateTask
        repo={repo}
        open={search.new === true}
        onClose={() => setSearch({ new: undefined } as never, true)}
      />
    </ListPane>
  );
}

function Groups({
  tasks,
  state,
  query,
  active,
}: {
  tasks: Task[];
  state?: State;
  query?: string;
  active?: string;
}) {
  const visible = tasks.filter(
    (task) =>
      (!state || status.task(task) === state) && matchesQuery(query, task.goal, task.id, task.tool),
  );
  if (visible.length === 0) return <Empty>No task matches the filter.</Empty>;
  return (
    <>
      {STATES.map((group) => {
        const rows = visible.filter((task) => status.task(task) === group);
        return rows.length === 0 ? null : (
          <Section
            key={group}
            title={STATE_LABELS[group]}
            count={rows.length}
            tone={group === "needs-you" ? "needs-you" : undefined}
          >
            <Rows label={STATE_LABELS[group]}>
              {rows.map((task) => (
                <TaskRow key={task.id} task={task} selected={task.id === active} />
              ))}
            </Rows>
          </Section>
        );
      })}
    </>
  );
}

function TaskRow({ task, selected }: { task: Task; selected: boolean }) {
  const state = status.task(task);
  return (
    <ItemRow
      state={state}
      title={task.goal}
      link={{ to: "/tasks/$id", params: { id: task.id }, search: true }}
      selected={selected}
      details={
        <>
          {state === "running" ? <span>{taskStep(task)}</span> : null}
          {task.error ? <span className="text-errored">{task.error}</span> : null}
        </>
      }
      meta={
        <>
          <ShortId id={task.id} />
          {task.tool ? <span className="hidden @md:inline">{task.tool}</span> : null}
        </>
      }
      time={
        state === "running" ? (
          <Timing startedAt={task.created_at} />
        ) : (
          <RelativeTime at={task.created_at} />
        )
      }
    />
  );
}

function CreateTask({ repo, open, onClose }: { repo: string; open: boolean; onClose: () => void }) {
  const [goal, setGoal] = useState("");
  const [tool, setTool] = useState("");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const toast = useToast();
  const create = useMutation({
    mutationFn: () => api.createTask(repo, goal.trim(), tool.trim() || undefined),
    onSuccess: async (task) => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.tasks(repo) });
      setGoal("");
      setTool("");
      toast.show({ title: "Task created", description: "The agent is starting." });
      await navigate({ to: "/tasks/$id", params: { id: task.id } });
    },
  });
  const cli = `igloo task create ${JSON.stringify(goal.trim() || "<goal>")}${tool.trim() ? ` --tool ${tool.trim()}` : ""} --repo ${repo}`;
  return (
    <Modal open={open} onOpenChange={(next) => !next && onClose()} label="Create task">
      <form
        aria-label="Create task"
        className="flex flex-col gap-4 p-5"
        onSubmit={(event) => {
          event.preventDefault();
          create.mutate();
        }}
      >
        <div className="flex flex-col gap-1">
          <h2 className="font-display text-lg font-semibold tracking-tight">Create task</h2>
          <p className="text-base text-muted-foreground">
            An agent works on the goal in its own sandbox and opens a change for you to review.
          </p>
        </div>
        <div className="flex flex-col gap-1.5 text-base font-medium">
          <label htmlFor="task-goal">Goal</label>
          <Textarea
            id="task-goal"
            required
            autoFocus
            rows={4}
            className="font-normal"
            placeholder="What should the agent do?"
            value={goal}
            onChange={(event) => setGoal(event.target.value)}
          />
        </div>
        <div className="flex flex-col gap-1.5 text-base font-medium">
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
          <p role="alert" className="text-base text-failed">
            {create.error.message}
          </p>
        ) : null}
        <CliCommand command={cli} />
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={!goal.trim() || create.isPending}>
            {create.isPending ? "Creating…" : "Create"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
