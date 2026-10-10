import { useQueries, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Box, Camera, Server } from "lucide-react";
import type { Run, Sandbox, Storage, Task, Worker } from "@/api/client";
import { Elapsed } from "@/components/elapsed";
import { EmptyState } from "@/components/empty-state";
import { Meter } from "@/components/meter";
import { Await, Empty, Page, Section } from "@/components/page";
import { RelativeTime } from "@/components/relative-time";
import { WithRepo } from "@/components/repo-page";
import { ShortId } from "@/components/short-id";
import { StatusPill } from "@/components/status-pill";
import { format } from "@/lib/format";
import { queries } from "@/lib/queries";
import { type State, status } from "@/lib/status";
import { cn } from "@/lib/utils";

/** System: the workers and their usage, and the sandboxes running on them. */
export function SystemPage({ sandbox }: { sandbox?: string }) {
  const workers = useQuery(queries.workers());
  const sandboxes = useQuery(queries.sandboxes());
  const storage = useQuery(queries.storage());
  return (
    <Page crumbs={[{ label: "System" }]} title="System">
      <Await query={workers} what="workers" rows={2}>
        {(workers) => (
          <Section title="Workers" count={workers.length}>
            {workers.length === 0 ? (
              <EmptyState
                icon={Server}
                title="No worker has connected."
                hint="The worker joins with IGLOO_WORKER_SERVER and IGLOO_WORKER_JOIN_TOKEN; docs/dev-linux.md has the whole command."
              >
                Workers run the sandboxes of tasks and checks. Each one shows its load, disk and
                layer cache here once it connects.
              </EmptyState>
            ) : (
              <div className="grid gap-4 @3xl:grid-cols-2">
                {workers.map((worker) => (
                  <WorkerCard key={worker.id} worker={worker} />
                ))}
              </div>
            )}
          </Section>
        )}
      </Await>
      <WithRepo>
        {(repo) => (
          <Await query={sandboxes} what="sandboxes">
            {(sandboxes) => (
              <>
                <Sandboxes repo={repo.id} sandboxes={sandboxes} focus={sandbox} />
                <Snapshots repo={repo.id} sandboxes={sandboxes} storage={storage.data} />
              </>
            )}
          </Await>
        )}
      </WithRepo>
      <Await query={storage} what="storage" rows={1}>
        {(storage) => <BlobStore storage={storage} />}
      </Await>
    </Page>
  );
}

const CONNECTION_LABELS: Record<Worker["connection"], string> = {
  connected: "Connected",
  disconnected: "Disconnected",
  lost: "Lost",
};

function WorkerCard({ worker }: { worker: Worker }) {
  const usage = worker.usage;
  return (
    <article
      aria-label={`Worker ${worker.id}`}
      className="flex flex-col gap-4 rounded-lg border p-4"
    >
      <header className="flex flex-wrap items-center gap-2">
        <StatusPill state={status.worker(worker)} label={CONNECTION_LABELS[worker.connection]} />
        <ShortId id={worker.id} />
        <span className="ml-auto text-sm text-muted-foreground">
          {worker.schedulability === "draining"
            ? "Draining: takes no new sandboxes"
            : "Schedulable"}
        </span>
      </header>
      <p className="flex flex-wrap items-center gap-1.5 text-sm text-muted-foreground">
        <span>
          {worker.capabilities.os}/{worker.capabilities.arch}, runtimes{" "}
          {worker.capabilities.runtimes.join(", ") || "none"}
        </span>
        {Object.entries(worker.labels).map(([key, value]) => (
          <span key={key} className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
            {key}={value}
          </span>
        ))}
      </p>
      {worker.disconnected_since ? (
        <p className="text-sm text-errored">
          Not connected since {format.time(worker.disconnected_since)}
        </p>
      ) : null}
      <dl className="grid grid-cols-3 gap-3 border-y py-3">
        <Figure label="Sandboxes" value={String(worker.allocated.sandboxes)} />
        <Figure label="CPU allocated" value={format.cpus(worker.allocated.millicpus)} />
        <Figure label="Memory allocated" value={format.mebibytes(worker.allocated.memory_mib)} />
      </dl>
      {usage ? (
        <div className="flex flex-col gap-3">
          <Meter
            label="Disk"
            value={usage.disk_total_bytes - usage.disk_free_bytes}
            max={usage.disk_total_bytes}
            text={`${format.bytes(usage.disk_total_bytes - usage.disk_free_bytes)} used of ${format.bytes(usage.disk_total_bytes)}`}
          />
          <Meter
            label="Layer cache"
            value={usage.layer_cache_bytes}
            max={usage.layer_cache_limit_bytes}
            text={`${format.bytes(usage.layer_cache_bytes)} of ${format.bytes(usage.layer_cache_limit_bytes)}`}
          />
          <p className="text-sm text-muted-foreground">
            Reported <RelativeTime at={usage.reported_at} />
          </p>
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">This worker has not reported its usage.</p>
      )}
    </article>
  );
}

function Figure({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <dt className="text-sm text-muted-foreground">{label}</dt>
      <dd className="tabular text-lg font-medium">{value}</dd>
    </div>
  );
}

type Owner = { kind: "task"; id: string } | { kind: "run"; id: string };

/**
 * Which task or run owns each sandbox. A task names its sandbox; a run does not, so the jobs of
 * the runs still in progress are read for theirs.
 */
function useOwners(repo: string): Map<string, Owner> {
  const tasks = useQuery(queries.tasks(repo));
  const runs = useQuery(queries.runs(repo));
  const active = (runs.data ?? []).filter((run) => status.run(run) === "running");
  const jobs = useQueries({
    queries: active.flatMap((run) =>
      [run.warm_job, ...run.checks.map((check) => check.job)]
        .filter((job): job is string => Boolean(job))
        .map((job) => queries.job(job)),
    ),
  });
  const owners = new Map<string, Owner>();
  for (const task of (tasks.data ?? []) as Task[]) {
    if (task.sandbox) owners.set(task.sandbox, { kind: "task", id: task.id });
  }
  const byJob = new Map<string, Run>();
  for (const run of active) {
    for (const job of [run.warm_job, ...run.checks.map((check) => check.job)]) {
      if (job) byJob.set(job, run);
    }
  }
  for (const { data } of jobs) {
    const run = data ? byJob.get(data.id) : undefined;
    if (data && run) owners.set(data.sandbox, { kind: "run", id: run.id });
  }
  return owners;
}

function Sandboxes({
  repo,
  sandboxes,
  focus,
}: {
  repo: string;
  sandboxes: Sandbox[];
  focus?: string;
}) {
  const owners = useOwners(repo);
  const live = sandboxes.filter((sandbox) => status.sandbox(sandbox) !== "closed");
  const ended = sandboxes.filter((sandbox) => status.sandbox(sandbox) === "closed");
  return (
    <>
      <Section title="Sandboxes" count={live.length}>
        {live.length === 0 ? (
          <EmptyState icon={Box} title="No sandbox is running.">
            A sandbox starts for each task and each run of checks, and shows its owner, limits and
            worker here.
          </EmptyState>
        ) : (
          <ul aria-label="Sandboxes" className="divide-y divide-border border-y">
            {live.map((sandbox) => (
              <SandboxRow
                key={sandbox.id}
                sandbox={sandbox}
                owner={owners.get(sandbox.id)}
                focused={sandbox.id === focus}
              />
            ))}
          </ul>
        )}
      </Section>
      {ended.length > 0 ? (
        <Section title="Stopped sandboxes" count={ended.length}>
          <ul aria-label="Stopped sandboxes" className="divide-y divide-border border-y">
            {ended.map((sandbox) => (
              <SandboxRow
                key={sandbox.id}
                sandbox={sandbox}
                owner={owners.get(sandbox.id)}
                focused={sandbox.id === focus}
              />
            ))}
          </ul>
        </Section>
      ) : null}
    </>
  );
}

function SandboxRow({
  sandbox,
  owner,
  focused,
}: {
  sandbox: Sandbox;
  owner?: Owner;
  focused: boolean;
}) {
  const state: State = status.sandbox(sandbox);
  return (
    <li
      aria-current={focused || undefined}
      className={cn(
        "grid grid-cols-[auto_minmax(0,1fr)] items-start gap-x-3 gap-y-1 px-4 py-3 @xl:grid-cols-[6rem_minmax(0,1fr)_auto]",
        focused && "bg-accent",
      )}
    >
      <StatusPill
        state={state}
        label={sandbox.phase.charAt(0).toUpperCase() + sandbox.phase.slice(1)}
        className="mt-px justify-self-start"
      />
      <div className="min-w-0 text-base">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-0.5">
          <ShortId id={sandbox.id} />
          {owner?.kind === "task" ? (
            <Link to="/tasks/$id" params={{ id: owner.id }} className="hover:underline">
              Task {format.shortId(owner.id)}
            </Link>
          ) : null}
          {owner?.kind === "run" ? (
            <Link to="/runs/$id" params={{ id: owner.id }} className="hover:underline">
              Run {format.shortId(owner.id)}
            </Link>
          ) : null}
          {!owner ? (
            <span className="text-muted-foreground">No task or run in progress</span>
          ) : null}
        </div>
        <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-sm text-muted-foreground">
          <span>
            {format.cpus(sandbox.limits.millicpus)}, {format.mebibytes(sandbox.limits.memory_mib)}
          </span>
          <span>{sandbox.isolation}</span>
          <span className="inline-flex items-center gap-1">
            snapshot <ShortId id={sandbox.snapshot} />
          </span>
          {sandbox.worker ? (
            <span className="inline-flex items-center gap-1">
              worker <ShortId id={sandbox.worker} />
            </span>
          ) : (
            <span>not placed on a worker</span>
          )}
          {sandbox.failure_reason ? (
            <span className="text-errored">{sandbox.failure_reason}</span>
          ) : null}
        </div>
      </div>
      <div className="col-start-2 text-sm text-muted-foreground @xl:col-start-3 @xl:text-right">
        {state === "running" ? (
          <Elapsed since={sandbox.created_at} />
        ) : (
          <RelativeTime at={sandbox.created_at} />
        )}
      </div>
    </li>
  );
}

/** What a snapshot row can say; `kind`, `size`, `builtAt` and `lastUsedAt` appear once the server records them. */
export type SnapshotRow = {
  id: string;
  sandboxes: number;
  kind?: "warm" | "agent";
  size?: number;
  builtAt?: string;
  lastUsedAt?: string;
};

/**
 * The repository's recorded snapshots, with when they were built and last used, and their size once the blob
 * store reports it, then any other snapshot a sandbox runs from; each with how many sandboxes
 * use it.
 */
function Snapshots({
  repo,
  sandboxes,
  storage,
}: {
  repo: string;
  sandboxes: Sandbox[];
  storage?: Storage;
}) {
  const recorded = useQuery(queries.repoSnapshots(repo));
  const sizes = new Map(
    (storage?.snapshots ?? []).map((snapshot) => [snapshot.snapshot, snapshot.size_bytes]),
  );
  const counts = new Map<string, number>();
  for (const sandbox of sandboxes)
    counts.set(sandbox.snapshot, (counts.get(sandbox.snapshot) ?? 0) + 1);
  const rows: SnapshotRow[] = (recorded.data ?? []).map((snapshot) => ({
    id: snapshot.snapshot,
    sandboxes: counts.get(snapshot.snapshot) ?? 0,
    size: snapshot.size_bytes ?? sizes.get(snapshot.snapshot) ?? undefined,
    builtAt: snapshot.built_at ?? undefined,
    lastUsedAt: snapshot.last_used_at ?? undefined,
  }));
  for (const [id, count] of counts) {
    if (!rows.some((row) => row.id === id)) rows.push({ id, sandboxes: count });
  }
  return (
    <Section title="Snapshots" count={rows.length}>
      {rows.length === 0 ? (
        <Empty>No snapshot has been built.</Empty>
      ) : (
        <table aria-label="Snapshots" className="w-full text-base">
          <thead>
            <tr className="border-b text-left text-sm text-muted-foreground">
              <th className="px-4 py-2 font-medium">
                <Camera className="mr-1.5 inline size-3.5" />
                Snapshot
              </th>
              <th className="px-4 py-2 font-medium">Kind</th>
              <th className="px-4 py-2 text-right font-medium">Size</th>
              <th className="px-4 py-2 text-right font-medium">Built</th>
              <th className="px-4 py-2 text-right font-medium">Last used</th>
              <th className="px-4 py-2 text-right font-medium">Sandboxes</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border">
            {rows.map((row) => (
              <tr key={row.id}>
                <td className="px-4 py-2">
                  <ShortId id={row.id} />
                </td>
                <td className="px-4 py-2 text-muted-foreground">{row.kind ?? "—"}</td>
                <td className="tabular px-4 py-2 text-right text-muted-foreground">
                  {row.size === undefined ? "—" : format.bytes(row.size)}
                </td>
                <td className="px-4 py-2 text-right text-muted-foreground">
                  {row.builtAt ? <RelativeTime at={row.builtAt} /> : "—"}
                </td>
                <td className="px-4 py-2 text-right text-muted-foreground">
                  {row.lastUsedAt ? <RelativeTime at={row.lastUsedAt} /> : "—"}
                </td>
                <td className="tabular px-4 py-2 text-right">{row.sandboxes}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </Section>
  );
}

/**
 * The server's blob store, where snapshot layers live: its size, and what the latest hourly
 * sweep kept and reclaimed.
 */
function BlobStore({ storage }: { storage: Storage }) {
  const sweep = storage.last_sweep;
  return (
    <Section title="Blob store">
      <div className="flex flex-col gap-3 rounded-lg border p-4">
        <dl className="grid grid-cols-3 gap-3">
          <Figure label="Layers and manifests" value={storage.blobs.toLocaleString()} />
          <Figure label="Size" value={format.bytes(storage.bytes)} />
          <div className="flex flex-col gap-0.5">
            <dt className="text-sm text-muted-foreground">Last sweep</dt>
            <dd className="text-lg font-medium">
              {sweep ? <RelativeTime at={sweep.at} /> : "None since the server started"}
            </dd>
          </div>
        </dl>
        {sweep ? (
          <p className="text-sm text-muted-foreground">
            Reclaimed{" "}
            <span className="tabular text-foreground">{format.bytes(sweep.reclaimed_bytes)}</span>{" "}
            in {sweep.reclaimed_blobs} blobs; kept {format.bytes(sweep.kept_bytes)} in{" "}
            {sweep.kept_blobs}. Unused layers older than a day are reclaimed every hour.
          </p>
        ) : null}
      </div>
    </Section>
  );
}
