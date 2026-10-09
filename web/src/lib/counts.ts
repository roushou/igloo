import { useQuery } from "@tanstack/react-query";
import { queries } from "@/lib/queries";
import { useCurrentRepo } from "@/lib/selected-repo";
import { status } from "@/lib/status";

/** The tasks Now reads; the sidebar's counts read the same queries, so they share one fetch. */
export const NOW_TASK_PHASES = ["awaiting_review", "preparing", "working"] as const;

/** What the sidebar counts: what waits on the user, and the changes still open. */
export function useNavCounts(): { waiting: number | null; openChanges: number | null } {
  const { repo } = useCurrentRepo();
  const id = repo?.id ?? "";
  const tasks = useQuery({ ...queries.tasks(id, [...NOW_TASK_PHASES]), enabled: Boolean(repo) });
  const changes = useQuery({ ...queries.changes(id, ["open"]), enabled: Boolean(repo) });
  const waiting =
    tasks.data && changes.data
      ? tasks.data.filter((task) => status.task(task) === "needs-you").length +
        changes.data.filter((change) => status.change(change) === "needs-you").length
      : null;
  return { waiting, openChanges: changes.data?.length ?? null };
}
