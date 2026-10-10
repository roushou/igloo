import { type QueryKey, queryOptions } from "@tanstack/react-query";
import { api, type ChangePhase, type TaskPhase, type Transcript } from "@/api/client";
import type { EventNotice } from "./event-stream";

/**
 * Query keys. A key is a prefix of the keys of the data it contains, so invalidating `tasks()`
 * refetches the task lists of every repository, and `task(id)` also covers that task's transcript.
 */
export const queryKeys = {
  repos: () => ["repos"] as const,
  repo: (id: string) => ["repo", id] as const,
  secrets: (repo: string) => ["repo", repo, "secrets"] as const,
  repoSnapshots: (repo: string) => ["repo", repo, "snapshots"] as const,
  tasks: (repo?: string | null) => (repo ? (["tasks", repo] as const) : (["tasks"] as const)),
  task: (id: string) => ["task", id] as const,
  transcript: (id: string) => ["task", id, "transcript"] as const,
  changes: (repo?: string | null) => (repo ? (["changes", repo] as const) : (["changes"] as const)),
  change: (id?: string) => (id ? (["change", id] as const) : (["change"] as const)),
  diff: (id: string, revision?: number) => ["change", id, "diff", revision ?? "latest"] as const,
  changeRuns: (id: string) => ["change", id, "runs"] as const,
  runs: (repo?: string | null) => (repo ? (["runs", repo] as const) : (["runs"] as const)),
  run: (id: string) => ["run", id] as const,
  job: (id: string) => ["job", id] as const,
  sandboxes: () => ["sandboxes"] as const,
  sandbox: (id: string) => ["sandbox", id] as const,
  workers: () => ["workers"] as const,
  storage: () => ["storage"] as const,
  workspaces: () => ["workspaces"] as const,
  workspace: (id: string) => ["workspace", id] as const,
  me: () => ["me"] as const,
};

/** How often a working task's transcript is read for new entries, in milliseconds. */
export const TRANSCRIPT_POLL_MS = 2_000;

/** Query definitions: the only place query keys are spelled. */
export const queries = {
  repos: () => queryOptions({ queryKey: queryKeys.repos(), queryFn: () => api.repos() }),

  secrets: (repo: string) =>
    queryOptions({ queryKey: queryKeys.secrets(repo), queryFn: () => api.secrets(repo) }),

  repoSnapshots: (repo: string) =>
    queryOptions({
      queryKey: queryKeys.repoSnapshots(repo),
      queryFn: () => api.repoSnapshots(repo),
    }),

  tasks: (repo: string, phases: TaskPhase[] = []) =>
    queryOptions({
      queryKey: [...queryKeys.tasks(repo), ...phases] as const,
      queryFn: () => api.tasks(repo, phases),
    }),

  task: (id: string) => queryOptions({ queryKey: queryKeys.task(id), queryFn: () => api.task(id) }),

  /**
   * A task's transcript. Each read asks only for the entries after the ones already cached and
   * appends them, and a working task is read again every `TRANSCRIPT_POLL_MS`.
   */
  transcript: (id: string) =>
    queryOptions({
      queryKey: queryKeys.transcript(id),
      queryFn: async ({ client }): Promise<Transcript> => {
        const known = client.getQueryData<Transcript>(queryKeys.transcript(id));
        const more = await api.transcript(id, known?.next ?? 0);
        if (!known) return more;
        return { ...more, entries: [...known.entries, ...more.entries] };
      },
      refetchInterval: (query) => (query.state.data?.idle === false ? TRANSCRIPT_POLL_MS : false),
    }),

  changes: (repo: string, phases: ChangePhase[] = []) =>
    queryOptions({
      queryKey: [...queryKeys.changes(repo), ...phases] as const,
      queryFn: () => api.changes(repo, phases),
    }),

  change: (id: string) =>
    queryOptions({ queryKey: queryKeys.change(id), queryFn: () => api.change(id) }),

  diff: (id: string, revision?: number) =>
    queryOptions({
      queryKey: queryKeys.diff(id, revision),
      queryFn: () => api.diff(id, revision),
    }),

  changeRuns: (id: string) =>
    queryOptions({ queryKey: queryKeys.changeRuns(id), queryFn: () => api.changeRuns(id) }),

  runs: (repo: string) =>
    queryOptions({ queryKey: queryKeys.runs(repo), queryFn: () => api.runs(repo) }),

  run: (id: string) => queryOptions({ queryKey: queryKeys.run(id), queryFn: () => api.run(id) }),

  job: (id: string) => queryOptions({ queryKey: queryKeys.job(id), queryFn: () => api.job(id) }),

  sandboxes: () =>
    queryOptions({ queryKey: queryKeys.sandboxes(), queryFn: () => api.sandboxes() }),

  workers: () => queryOptions({ queryKey: queryKeys.workers(), queryFn: () => api.workers() }),

  storage: () => queryOptions({ queryKey: queryKeys.storage(), queryFn: () => api.storage() }),

  /** The signed-in person's workspaces. */
  workspaces: () =>
    queryOptions({ queryKey: queryKeys.workspaces(), queryFn: () => api.workspaces() }),

  workspace: (id: string) =>
    queryOptions({ queryKey: queryKeys.workspace(id), queryFn: () => api.workspace(id) }),

  /** Who the signed-in token stands for; it does not change while signed in. */
  me: () =>
    queryOptions({ queryKey: queryKeys.me(), queryFn: () => api.me(), staleTime: Infinity }),
};

/**
 * The queries an event makes stale: its resource, the lists it appears in, and for runs the
 * changes whose merge readiness they decide. Events of resources the console does not show
 * make nothing stale.
 */
export function staleQueries(notice: EventNotice): QueryKey[] {
  const { resource_type: type, resource_id: id, repo } = notice;
  switch (type) {
    case "repo":
      return [queryKeys.repos(), queryKeys.repo(id)];
    case "task":
      return [queryKeys.task(id), queryKeys.tasks(repo)];
    case "change":
      return [queryKeys.change(id), queryKeys.changes(repo)];
    case "run":
      return [queryKeys.run(id), queryKeys.runs(repo), queryKeys.changes(repo), queryKeys.change()];
    case "job":
      return [queryKeys.job(id)];
    case "sandbox":
      return [queryKeys.sandbox(id), queryKeys.sandboxes()];
    case "worker":
      return [queryKeys.workers()];
    case "workspace":
      return [queryKeys.workspace(id), queryKeys.workspaces()];
    default:
      return [];
  }
}
