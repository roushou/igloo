import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api, type Workspace } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Modal, ModalBody } from "@/components/ui/dialog";
import { useToast } from "@/components/ui/toast";
import { queryKeys } from "@/lib/queries";

/** What a workspace's buttons may do in its phase: the rule that is not met, or `null`. */
export const workspaceRules = {
  start: (workspace: Pick<Workspace, "phase">): string | null =>
    workspace.phase === "stopped" ? null : "The workspace is not stopped",
  stop: (workspace: Pick<Workspace, "phase">): string | null =>
    workspace.phase === "starting" || workspace.phase === "running"
      ? null
      : workspace.phase === "stopping"
        ? "The workspace is already stopping"
        : "The workspace is already stopped",
};

/**
 * The start, stop and delete of one workspace, each toasting its outcome and refreshing the
 * lists. `onDeleted` runs after a delete the server accepted.
 */
export function useWorkspaceActions(workspace: Workspace, onDeleted?: () => void) {
  const queryClient = useQueryClient();
  const toast = useToast();
  const refresh = async (updated?: Workspace) => {
    if (updated) queryClient.setQueryData(queryKeys.workspace(workspace.id), updated);
    await queryClient.invalidateQueries({ queryKey: queryKeys.workspaces() });
  };
  const failed = (title: string) => (error: Error) =>
    toast.show({ title, description: error.message, tone: "error" });

  const start = useMutation({
    mutationFn: () => api.startWorkspace(workspace.id),
    onSuccess: async (updated) => {
      await refresh(updated);
      toast.show({
        title: "Workspace starting",
        description: "It resumes from what the last stop sealed.",
      });
    },
    onError: failed("Could not start the workspace"),
  });
  const stop = useMutation({
    mutationFn: () => api.stopWorkspace(workspace.id),
    onSuccess: async (updated) => {
      await refresh(updated);
      toast.show({
        title: "Workspace stopping",
        description: "Its changes are sealed before the sandbox stops.",
      });
    },
    onError: failed("Could not stop the workspace"),
  });
  const remove = useMutation({
    mutationFn: () => api.deleteWorkspace(workspace.id),
    onSuccess: async () => {
      queryClient.removeQueries({ queryKey: queryKeys.workspace(workspace.id) });
      await refresh();
      toast.show({ title: "Workspace deleted", description: `Branch ${workspace.branch}.` });
      onDeleted?.();
    },
    onError: failed("Could not delete the workspace"),
  });
  return { start, stop, remove };
}

/** Asks before a workspace is deleted: what it holds that no stop sealed is lost. */
export function DeleteWorkspaceDialog({
  workspace,
  open,
  pending,
  onCancel,
  onDelete,
}: {
  workspace: Pick<Workspace, "branch">;
  open: boolean;
  pending: boolean;
  onCancel: () => void;
  onDelete: () => void;
}) {
  return (
    <Modal
      open={open}
      onOpenChange={(next) => !next && onCancel()}
      label="Delete this workspace"
      role="alertdialog"
    >
      <ModalBody
        title="Delete this workspace?"
        description={
          <>
            The workspace on <span className="font-medium text-foreground">{workspace.branch}</span>{" "}
            and everything in it is dropped, including what a stop has not sealed. Pushed branches
            stay.
          </>
        }
      >
        <Button variant="ghost" onClick={onCancel}>
          Keep it
        </Button>
        <Button variant="danger" disabled={pending} onClick={onDelete}>
          {pending ? "Deleting…" : "Delete"}
        </Button>
      </ModalBody>
    </Modal>
  );
}
