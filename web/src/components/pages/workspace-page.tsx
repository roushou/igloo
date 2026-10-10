import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { ArrowUpRight, GitPullRequest, Play, Square, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import { ApiError, type Change, type Workspace } from "@/api/client";
import { ActionButton, CliMenu } from "@/components/action-button";
import { latestRun } from "@/components/change-checks";
import { ChecksStrip } from "@/components/checks-strip";
import { Await, ErrorNote, Loading, PageHeader } from "@/components/page";
import { RelativeTime } from "@/components/relative-time";
import { ShortId } from "@/components/short-id";
import { StatusPill } from "@/components/status-pill";
import { Terminal } from "@/components/terminal";
import { Button } from "@/components/ui/button";
import {
  DeleteWorkspaceDialog,
  useWorkspaceActions,
  workspaceRules,
} from "@/components/workspace-actions";
import { type Command, useRegisterCommands } from "@/lib/commands";
import { queries } from "@/lib/queries";
import { waitingOn } from "@/lib/readiness";
import { status, WORKSPACE_LABELS } from "@/lib/status";
import { runStep } from "@/lib/steps";

const CRUMBS = [{ label: "Workspaces", link: { to: "/workspaces" as const } }];

/**
 * A workspace: its terminal over the full height, and beside it the branch's change, the checks
 * of its latest revision and the pushes that made its revisions.
 */
export function WorkspacePage({ id }: { id: string }) {
  const workspace = useQuery(queries.workspace(id));
  if (workspace.error instanceof ApiError && workspace.error.status === 404) {
    return <Deleted />;
  }
  return (
    <Await query={workspace} what="the workspace">
      {(workspace) => <WorkspaceDetail workspace={workspace} />}
    </Await>
  );
}

/** What a workspace that no longer exists shows. */
function Deleted() {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader crumbs={[...CRUMBS, { label: "Deleted" }]} />
      <p role="status" className="p-8 text-base text-muted-foreground">
        This workspace was deleted.{" "}
        <Link to="/workspaces" className="text-foreground underline">
          Back to your workspaces
        </Link>
      </p>
    </div>
  );
}

function WorkspaceDetail({ workspace }: { workspace: Workspace }) {
  const navigate = useNavigate();
  const [asking, setAsking] = useState(false);
  const { start, stop, remove } = useWorkspaceActions(workspace, () => {
    void navigate({ to: "/workspaces" });
  });
  const startUnmet = workspaceRules.start(workspace);
  const stopUnmet = workspaceRules.stop(workspace);

  const commands = useMemo<Command[]>(
    () => [
      {
        id: "start-workspace",
        label: "Start this workspace",
        group: "This workspace",
        icon: Play,
        unmet: startUnmet,
        run: () => start.mutate(),
      },
      {
        id: "stop-workspace",
        label: "Stop this workspace",
        group: "This workspace",
        icon: Square,
        unmet: stopUnmet,
        run: () => stop.mutate(),
      },
      {
        id: "delete-workspace",
        label: "Delete this workspace",
        group: "This workspace",
        icon: Trash2,
        run: () => setAsking(true),
      },
    ],
    [startUnmet, stopUnmet, start.mutate, stop.mutate],
  );
  useRegisterCommands(commands);

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
      <PageHeader
        crumbs={[...CRUMBS, { label: <span className="font-mono">{workspace.branch}</span> }]}
        actions={
          <>
            <CliMenu
              commands={[
                { label: "Open its terminal", command: `igloo shell ${workspace.id}` },
                { label: "Start", command: `igloo workspace start ${workspace.id}` },
                { label: "Stop", command: `igloo workspace stop ${workspace.id}` },
                { label: "Delete", command: `igloo workspace delete ${workspace.id}` },
              ]}
            />
            {startUnmet ? (
              <ActionButton
                label="Stop"
                unmet={stopUnmet}
                pending={stop.isPending}
                onRun={() => stop.mutate()}
              />
            ) : (
              <ActionButton label="Start" pending={start.isPending} onRun={() => start.mutate()} />
            )}
            <Button variant="ghost" onClick={() => setAsking(true)}>
              Delete
            </Button>
          </>
        }
      />
      <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
        <main className="flex min-h-80 min-w-0 flex-1 flex-col gap-3 p-3 sm:p-4">
          <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-base text-muted-foreground">
            <StatusPill
              state={status.workspace(workspace)}
              label={WORKSPACE_LABELS[workspace.phase]}
            />
            <span>
              Last used <RelativeTime at={workspace.last_activity} />
            </span>
            <ShortId id={workspace.id} />
          </div>
          <SetupNote workspace={workspace} />
          <Screen workspace={workspace} onStart={() => start.mutate()} />
        </main>
        <aside
          aria-label="Branch"
          className="flex w-full shrink-0 flex-col gap-6 overflow-y-auto border-t p-4 lg:w-80 lg:border-t-0 lg:border-l"
        >
          <BranchPanel workspace={workspace} />
        </aside>
      </div>
      <DeleteWorkspaceDialog
        workspace={workspace}
        open={asking}
        pending={remove.isPending}
        onCancel={() => setAsking(false)}
        onDelete={() => remove.mutate(undefined, { onSettled: () => setAsking(false) })}
      />
    </div>
  );
}

/** A failed setup: the workspace works, but its dotfiles may not be in place. */
function SetupNote({ workspace }: { workspace: Workspace }) {
  const setup = workspace.setup;
  if (setup?.state !== "failed") return null;
  return (
    <div
      role="alert"
      className="rounded-lg border border-errored/30 bg-errored-soft px-4 py-3 text-base text-errored"
    >
      <p className="font-medium">The workspace's setup failed.</p>
      <p className="mt-0.5">
        {setup.reason ? `${setup.reason}. ` : null}
        The terminal still works; your dotfiles may be missing.{" "}
        <Link to="/jobs/$id" params={{ id: setup.job }} className="underline">
          Open the setup log
        </Link>
      </p>
    </div>
  );
}

/** The terminal while the workspace runs, and what stands in for it in every other phase. */
function Screen({ workspace, onStart }: { workspace: Workspace; onStart: () => void }) {
  if (workspace.phase === "running" && workspace.sandbox) {
    return (
      <Terminal key={workspace.sandbox} sandboxId={workspace.sandbox} className="min-h-0 flex-1" />
    );
  }
  const text = {
    starting: "The sandbox is starting. The terminal opens when it runs.",
    running: "The sandbox is starting. The terminal opens when it runs.",
    stopping: "Sealing your changes, then stopping the sandbox.",
    stopped: "Stopped. Start it to resume from what the last stop sealed.",
  }[workspace.phase];
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 rounded-md bg-code p-6 text-center text-code-muted">
      <p role="status">{text}</p>
      {workspace.phase === "stopped" ? (
        <Button variant="outline" onClick={onStart}>
          Start
        </Button>
      ) : null}
    </div>
  );
}

const PUSHES_SHOWN = 5;

/** The change proposing the workspace's branch: the open one, else the newest. */
function branchChange(changes: readonly Change[], branch: string): Change | null {
  const mine = changes.filter((change) => change.source_branch === branch);
  return mine.find((change) => change.phase === "open") ?? mine[0] ?? null;
}

function BranchPanel({ workspace }: { workspace: Workspace }) {
  const changes = useQuery(queries.changes(workspace.repo));
  return (
    <Await query={changes} what="the branch's change" rows={3}>
      {(changes) => {
        const change = branchChange(changes, workspace.branch);
        return change ? (
          <>
            <ChangeBlock change={change} />
            <ChecksBlock change={change} />
            <PushesBlock change={change} />
          </>
        ) : (
          <>
            <PanelSection title="Change">
              <p className="text-base text-muted-foreground">
                No change proposes {workspace.branch}. Push the branch to Igloo and one opens.
              </p>
            </PanelSection>
            <PanelSection title="Recent pushes">
              <p className="text-base text-muted-foreground">Nothing pushed yet.</p>
            </PanelSection>
          </>
        );
      }}
    </Await>
  );
}

function PanelSection({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section aria-label={title} className="flex flex-col gap-2">
      <h2 className="font-sans text-base font-semibold">{title}</h2>
      {children}
    </section>
  );
}

function ChangeBlock({ change }: { change: Change }) {
  return (
    <PanelSection title="Change">
      <Link
        to="/changes/$id"
        params={{ id: change.id }}
        className="flex flex-col gap-1.5 rounded-lg border px-3 py-2.5 text-base hover:bg-accent/60"
      >
        <span className="flex items-center gap-2">
          <GitPullRequest className="size-4 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1 truncate font-medium">{change.title}</span>
        </span>
        <span className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
          <StatusPill state={status.change(change)} />
          {change.phase === "open" ? <span>{waitingOn(change)}</span> : null}
        </span>
      </Link>
    </PanelSection>
  );
}

function ChecksBlock({ change }: { change: Change }) {
  const revision = change.revisions.at(-1)?.number ?? 0;
  const runs = useQuery(queries.changeRuns(change.id));
  return (
    <PanelSection title="Checks">
      {runs.isPending ? (
        <Loading what="the checks" rows={1} />
      ) : runs.isError ? (
        <ErrorNote error={runs.error} what="the checks" onRetry={runs.refetch} />
      ) : (
        <Run run={latestRun(runs.data, revision)} revision={revision} />
      )}
    </PanelSection>
  );
}

function Run({ run, revision }: { run: ReturnType<typeof latestRun>; revision: number }) {
  if (!run) {
    return (
      <p className="text-base text-muted-foreground">No checks have run for revision {revision}.</p>
    );
  }
  return (
    <div className="flex flex-col gap-2 text-base">
      <div className="flex flex-wrap items-center gap-2">
        <StatusPill state={status.run(run)} />
        <ChecksStrip checks={run.checks} />
      </div>
      <p className="text-sm text-muted-foreground">
        {runStep(run)}, started <RelativeTime at={run.started_at} />
      </p>
      <Link
        to="/runs/$id"
        params={{ id: run.id }}
        className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
      >
        Open the run
        <ArrowUpRight className="size-3.5" />
      </Link>
    </div>
  );
}

function PushesBlock({ change }: { change: Change }) {
  const pushes = [...change.revisions].reverse().slice(0, PUSHES_SHOWN);
  return (
    <PanelSection title="Recent pushes">
      <ol aria-label="Recent pushes" className="flex flex-col gap-1.5">
        {pushes.map((revision) => (
          <li key={revision.number} className="flex items-baseline gap-2 text-base">
            <span>Revision {revision.number}</span>
            <span className="font-mono text-sm text-muted-foreground">
              {revision.head.slice(0, 7)}
            </span>
            <RelativeTime
              at={revision.created_at}
              className="tabular ml-auto text-sm text-muted-foreground"
            />
          </li>
        ))}
      </ol>
    </PanelSection>
  );
}
