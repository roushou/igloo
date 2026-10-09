import { useQueries, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { Run, Sandbox, Task, Worker } from "@/api/client";
import { Elapsed } from "@/components/elapsed";
import { Meter } from "@/components/meter";
import { Await, Empty, Page, Rows, Section } from "@/components/page";
import { WithRepo } from "@/components/repo-page";
import { ShortId } from "@/components/short-id";
import { StatusPill } from "@/components/status-pill";
import { format } from "@/lib/format";
import { queries } from "@/lib/queries";
import { type State, status } from "@/lib/status";
import { useNow } from "@/lib/use-now";
import { cn } from "@/lib/utils";

/** System: the workers and their usage, and the sandboxes running on them. */
export function SystemPage({ sandbox }: { sandbox?: string }) {
  const workers = useQuery(queries.workers());
  const sandboxes = useQuery(queries.sandboxes());
  return (
    <Page title="System">
      <Await query={workers} what="workers">
        {(workers) => (
          <Section title="Workers" count={workers.length}>
            {workers.length === 0 ? (
              <Empty>No worker has connected.</Empty>
            ) : (
              <div className="grid gap-4 lg:grid-cols-2">
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
            {(sandboxes) => <Sandboxes repo={repo.id} sandboxes={sandboxes} focus={sandbox} />}
          </Await>
        )}
      </WithRepo>
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
  const now = useNow(5_000);
  return (
    <article
      aria-label={`Worker ${worker.id}`}
      className="flex flex-col gap-3 rounded-lg border bg-card p-4"
    >
      <header className="flex flex-wrap items-center gap-2">
        <StatusPill state={status.worker(worker)} label={CONNECTION_LABELS[worker.connection]} />
        <ShortId id={worker.id} />
        <span className="text-xs text-muted-foreground">
          {worker.schedulability === "draining"
            ? "Draining: takes no new sandboxes"
            : "Schedulable"}
        </span>
      </header>
      <p className="text-xs text-muted-foreground">
        {worker.capabilities.os}/{worker.capabilities.arch}, runtimes{" "}
        {worker.capabilities.runtimes.join(", ") || "none"}
        {Object.entries(worker.labels).map(([key, value]) => (
          <span key={key} className="ml-2 rounded bg-muted px-1.5 py-0.5 font-mono">
            {key}={value}
          </span>
        ))}
      </p>
      {worker.disconnected_since ? (
        <p className="text-xs text-errored">
          Not connected since {format.time(worker.disconnected_since)}
        </p>
      ) : null}
      <dl className="grid grid-cols-3 gap-2 text-sm">
        <Figure label="Sandboxes" value={String(worker.allocated.sandboxes)} />
        <Figure label="CPU allocated" value={format.cpus(worker.allocated.millicpus)} />
        <Figure label="Memory allocated" value={format.mebibytes(worker.allocated.memory_mib)} />
      </dl>
      {usage ? (
        <div className="flex flex-col gap-2">
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
          <p className="text-xs text-muted-foreground">
            Reported {format.duration(now - new Date(usage.reported_at).getTime())} ago
          </p>
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">This worker has not reported its usage.</p>
      )}
    </article>
  );
}

function Figure({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="tabular">{value}</dd>
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
          <Empty>No sandbox is running.</Empty>
        ) : (
          <Rows label="Sandboxes">
            {live.map((sandbox) => (
              <SandboxRow
                key={sandbox.id}
                sandbox={sandbox}
                owner={owners.get(sandbox.id)}
                focused={sandbox.id === focus}
              />
            ))}
          </Rows>
        )}
      </Section>
      {ended.length > 0 ? (
        <Section title="Stopped sandboxes" count={ended.length}>
          <Rows label="Stopped sandboxes">
            {ended.map((sandbox) => (
              <SandboxRow
                key={sandbox.id}
                sandbox={sandbox}
                owner={owners.get(sandbox.id)}
                focused={sandbox.id === focus}
              />
            ))}
          </Rows>
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
      className={cn("flex items-start gap-3 px-4 py-3", focused && "bg-muted")}
    >
      <StatusPill
        state={state}
        label={sandbox.phase.charAt(0).toUpperCase() + sandbox.phase.slice(1)}
        className="mt-0.5 w-24 justify-start"
      />
      <div className="min-w-0 flex-1 text-sm">
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
        <div className="mt-0.5 flex flex-wrap gap-x-3 text-xs text-muted-foreground">
          <span>
            {format.cpus(sandbox.limits.millicpus)}, {format.mebibytes(sandbox.limits.memory_mib)}
          </span>
          <span>{sandbox.isolation}</span>
          <span>
            snapshot <ShortId id={sandbox.snapshot} />
          </span>
          {sandbox.worker ? (
            <span>
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
      {state === "running" ? (
        <div className="shrink-0 text-xs text-muted-foreground">
          <Elapsed since={sandbox.created_at} />
        </div>
      ) : null}
    </li>
  );
}
