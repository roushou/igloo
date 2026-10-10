import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Plus, SquareTerminal } from "lucide-react";
import { useState } from "react";
import { api, type Repo, type Workspace } from "@/api/client";
import { ActionButton, CliCommand, CliMenu } from "@/components/action-button";
import { EmptyState } from "@/components/empty-state";
import { ItemRow } from "@/components/item-row";
import { Await, Page, Rows } from "@/components/page";
import { RelativeTime } from "@/components/relative-time";
import { ShortId } from "@/components/short-id";
import { Button } from "@/components/ui/button";
import { Modal } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { useToast } from "@/components/ui/toast";
import {
  DeleteWorkspaceDialog,
  useWorkspaceActions,
  workspaceRules,
} from "@/components/workspace-actions";
import { format } from "@/lib/format";
import { queries, queryKeys } from "@/lib/queries";
import { useCurrentRepo } from "@/lib/selected-repo";
import { status, WORKSPACE_LABELS } from "@/lib/status";

/** Workspaces: the signed-in person's, in every repository, with open, stop, delete and create. */
export function WorkspacesPage() {
  const { repo } = useCurrentRepo();
  const [creating, setCreating] = useState(false);
  const workspaces = useQuery(queries.workspaces());
  const repos = useQuery(queries.repos());
  const names = new Map((repos.data ?? []).map((r) => [r.id, format.repoName(r.location)]));
  return (
    <Page
      crumbs={[{ label: "Workspaces" }]}
      title="Workspaces"
      actions={
        <>
          <CliMenu
            commands={[
              {
                label: "Create a workspace",
                command: `igloo workspace create --repo ${repo?.id ?? "<repo id>"} --branch <branch>`,
              },
              { label: "List workspaces", command: "igloo workspace list" },
            ]}
          />
          <ActionButton
            label="Create workspace"
            variant="default"
            icon={<Plus />}
            unmet={repo ? null : "No repository is registered"}
            onRun={() => setCreating(true)}
          />
        </>
      }
    >
      <Await query={workspaces} what="workspaces">
        {(list) =>
          list.length === 0 ? (
            <EmptyState
              icon={SquareTerminal}
              title="No workspace yet"
              command={`igloo workspace create --repo ${repo?.id ?? "<repo id>"} --branch main`}
            >
              A workspace is your own sandbox on a branch, with a terminal. What you leave in it
              survives a stop.
            </EmptyState>
          ) : (
            <Rows label="Workspaces">
              {list.map((workspace) => (
                <WorkspaceRow
                  key={workspace.id}
                  workspace={workspace}
                  repoName={names.get(workspace.repo)}
                />
              ))}
            </Rows>
          )
        }
      </Await>
      {repo ? (
        <CreateWorkspace repo={repo} open={creating} onClose={() => setCreating(false)} />
      ) : null}
    </Page>
  );
}

function WorkspaceRow({ workspace, repoName }: { workspace: Workspace; repoName?: string }) {
  const { start, stop, remove } = useWorkspaceActions(workspace);
  const [asking, setAsking] = useState(false);
  const startUnmet = workspaceRules.start(workspace);
  const stopUnmet = workspaceRules.stop(workspace);
  return (
    <>
      <ItemRow
        state={status.workspace(workspace)}
        label={WORKSPACE_LABELS[workspace.phase]}
        title={workspace.branch}
        link={{ to: "/workspaces/$id", params: { id: workspace.id } }}
        details={
          <>
            <span>{repoName ?? format.shortId(workspace.repo)}</span>
            {workspace.setup?.state === "failed" ? (
              <span className="text-errored">Setup failed</span>
            ) : null}
          </>
        }
        meta={<ShortId id={workspace.id} />}
        time={<RelativeTime at={workspace.last_activity} />}
        actions={
          <>
            {startUnmet ? (
              <ActionButton
                label="Stop"
                size="sm"
                unmet={stopUnmet}
                pending={stop.isPending}
                onRun={() => stop.mutate()}
              />
            ) : (
              <ActionButton
                label="Start"
                size="sm"
                pending={start.isPending}
                onRun={() => start.mutate()}
              />
            )}
            <Button variant="ghost" size="sm" onClick={() => setAsking(true)}>
              Delete
            </Button>
          </>
        }
      />
      <DeleteWorkspaceDialog
        workspace={workspace}
        open={asking}
        pending={remove.isPending}
        onCancel={() => setAsking(false)}
        onDelete={() => remove.mutate(undefined, { onSettled: () => setAsking(false) })}
      />
    </>
  );
}

function CreateWorkspace({
  repo,
  open,
  onClose,
}: {
  repo: Repo;
  open: boolean;
  onClose: () => void;
}) {
  const [branch, setBranch] = useState("");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const toast = useToast();
  const create = useMutation({
    mutationFn: () => api.createWorkspace(repo.id, branch.trim() || undefined),
    onSuccess: async (workspace) => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.workspaces() });
      setBranch("");
      onClose();
      toast.show({ title: "Workspace created", description: "Its sandbox is starting." });
      await navigate({ to: "/workspaces/$id", params: { id: workspace.id } });
    },
  });
  const cli = `igloo workspace create --repo ${repo.id} --branch ${branch.trim() || repo.default_branch}`;
  return (
    <Modal open={open} onOpenChange={(next) => !next && onClose()} label="Create workspace">
      <form
        aria-label="Create workspace"
        className="flex flex-col gap-4 p-5"
        onSubmit={(event) => {
          event.preventDefault();
          create.mutate();
        }}
      >
        <div className="flex flex-col gap-1">
          <h2 className="font-display text-lg font-semibold tracking-tight">Create workspace</h2>
          <p className="text-base text-muted-foreground">
            A sandbox of {format.repoName(repo.location)} at the head of a branch, with a terminal.
          </p>
        </div>
        <div className="flex flex-col gap-1.5 text-base font-medium">
          <label htmlFor="workspace-branch">Branch</label>
          <Input
            id="workspace-branch"
            autoFocus
            className="font-normal"
            placeholder={`${repo.default_branch} (the default branch)`}
            value={branch}
            onChange={(event) => setBranch(event.target.value)}
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
          <Button type="submit" disabled={create.isPending}>
            {create.isPending ? "Creating…" : "Create"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
