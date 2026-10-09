import { type QueryKey, queryOptions } from "@tanstack/react-query";
import { api } from "@/api/client";
import type { EventNotice } from "./event-stream";

/**
 * Query keys. A key is a prefix of the keys of the data it contains, so invalidating `tasks()`
 * refetches the task lists of every repository.
 */
export const queryKeys = {
  repos: () => ["repos"] as const,
  repo: (id: string) => ["repo", id] as const,
  tasks: (repo?: string | null) => (repo ? (["tasks", repo] as const) : (["tasks"] as const)),
  task: (id: string) => ["task", id] as const,
  changes: (repo?: string | null) => (repo ? (["changes", repo] as const) : (["changes"] as const)),
  change: (id: string) => ["change", id] as const,
  runs: (repo?: string | null) => (repo ? (["runs", repo] as const) : (["runs"] as const)),
  run: (id: string) => ["run", id] as const,
  job: (id: string) => ["job", id] as const,
  sandbox: (id: string) => ["sandbox", id] as const,
  workers: () => ["workers"] as const,
};

/** Query definitions: the only place query keys are spelled. */
export const queries = {
  repos: () => queryOptions({ queryKey: queryKeys.repos(), queryFn: () => api.repos() }),
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
      return [queryKeys.run(id), queryKeys.runs(repo), queryKeys.changes(repo)];
    case "job":
      return [queryKeys.job(id)];
    case "sandbox":
      return [queryKeys.sandbox(id)];
    case "worker":
      return [queryKeys.workers()];
    default:
      return [];
  }
}
