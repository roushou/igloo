import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { lazy, Suspense, useState } from "react";
import { api, type Change } from "@/api/client";
import { ActionButton } from "@/components/action-button";
import { ChecksTab } from "@/components/change-checks";
import { CommentsTab } from "@/components/change-comments";
import { TimelineTab } from "@/components/change-timeline";
import { Await, Fact, Loading, Page } from "@/components/page";
import { ShortId } from "@/components/short-id";
import { StatusPill } from "@/components/status-pill";
import { changeCli } from "@/lib/cli";
import { queries, queryKeys } from "@/lib/queries";
import { mergeBlocker, mergeRules } from "@/lib/readiness";
import { status } from "@/lib/status";
import { cn } from "@/lib/utils";

// The diff library is loaded when the tab opens, so building and prerendering never evaluate it.
const DiffView = lazy(() => import("@/components/diff-view"));

const TABS = ["Diff", "Checks", "Comments", "Timeline"] as const;
type Tab = (typeof TABS)[number];

/** A change: its merge checklist, the actions on it, and the Diff, Checks, Comments and Timeline. */
export function ChangePage({ id }: { id: string }) {
  const change = useQuery(queries.change(id));
  return (
    <Await query={change} what="the change">
      {(change) => <ChangeDetail change={change} />}
    </Await>
  );
}

function ChangeDetail({ change }: { change: Change }) {
  const latest = change.revisions.at(-1)?.number ?? 1;
  const [picked, setPicked] = useState<number | null>(null);
  const revision = picked ?? latest;
  const [tab, setTab] = useState<Tab>("Diff");
  const state = status.change(change);
  const rules = mergeRules(change);

  return (
    <Page
      title={change.title}
      meta={
        <>
          <StatusPill state={state} />
          <span className="font-mono text-xs">
            {change.source_branch} → {change.target_branch}
          </span>
          <Fact label="Change">
            <ShortId id={change.id} />
          </Fact>
          {change.merged_commit ? (
            <Fact label="Merged as">
              <ShortId id={change.merged_commit} />
            </Fact>
          ) : null}
          <label className="inline-flex items-center gap-1.5">
            <span className="text-muted-foreground">Revision</span>
            <select
              aria-label="Revision"
              className="h-7 rounded-md border bg-transparent px-1.5 text-sm"
              value={revision}
              onChange={(event) => setPicked(Number(event.target.value))}
            >
              {change.revisions.map((entry) => (
                <option key={entry.number} value={entry.number}>
                  {entry.number}
                  {entry.number === latest ? " (latest)" : ""}
                </option>
              ))}
            </select>
          </label>
          <Fact label="Head">
            <ShortId id={change.revisions.find((r) => r.number === revision)?.head ?? ""} />
          </Fact>
        </>
      }
      actions={<Actions change={change} />}
    >
      {rules.length > 0 ? (
        <ul
          aria-label="Merge checklist"
          className="flex flex-col gap-1.5 rounded-lg border bg-card px-4 py-3 text-sm"
        >
          {rules.map((rule) => (
            <li
              key={rule.id}
              data-state={rule.state}
              data-met={rule.met}
              className="flex items-center gap-2"
            >
              <StatusDot state={rule.state} />
              <span
                className={cn(
                  !rule.met && rule.state === "needs-you" && "font-medium text-expedition",
                )}
              >
                {rule.text}
              </span>
            </li>
          ))}
        </ul>
      ) : null}

      <div role="tablist" aria-label="Change" className="flex gap-1 border-b">
        {TABS.map((name) => (
          <button
            key={name}
            type="button"
            role="tab"
            aria-selected={tab === name}
            className={cn(
              "-mb-px border-b-2 border-transparent px-3 py-2 text-sm text-muted-foreground hover:text-foreground",
              tab === name && "border-foreground font-medium text-foreground",
            )}
            onClick={() => setTab(name)}
          >
            {name}
            {name === "Comments" && (change.comments?.length ?? 0) > 0 ? (
              <span className="tabular ml-1.5 text-xs">{change.comments?.length}</span>
            ) : null}
          </button>
        ))}
      </div>
      <div role="tabpanel" aria-label={tab}>
        {tab === "Diff" ? (
          <Suspense fallback={<Loading what="the diff" />}>
            <DiffView change={change} revision={revision} />
          </Suspense>
        ) : null}
        {tab === "Checks" ? <ChecksTab changeId={change.id} revision={revision} /> : null}
        {tab === "Comments" ? <CommentsTab change={change} revision={revision} /> : null}
        {tab === "Timeline" ? <TimelineTab change={change} /> : null}
      </div>
    </Page>
  );
}

function StatusDot({ state }: { state: string }) {
  const color: Record<string, string> = {
    "needs-you": "bg-expedition",
    running: "bg-running",
    passed: "bg-passed",
    failed: "bg-failed",
    errored: "bg-errored",
    closed: "bg-closed",
  };
  return <span aria-hidden className={cn("size-2 shrink-0 rounded-full", color[state])} />;
}

/** The actions on a change. Each is disabled with the rule it needs, and shows its CLI command. */
function Actions({ change }: { change: Change }) {
  const queryClient = useQueryClient();
  const run = useMutation({
    mutationFn: (action: (id: string) => Promise<Change>) => action(change.id),
    onSuccess: async (updated) => {
      queryClient.setQueryData(queryKeys.change(change.id), updated);
      await queryClient.invalidateQueries({ queryKey: queryKeys.changes(change.repo) });
      await queryClient.invalidateQueries({ queryKey: queryKeys.changeRuns(change.id) });
    },
  });
  const open = change.phase === "open";
  const ended = open ? null : `The change is already ${change.phase}`;
  const latest = change.revisions.at(-1)?.number;
  const approved =
    change.readiness?.approval.state === "given" ||
    change.approvals.some((approval) => approval.revision === latest);
  const needsApproval = change.readiness?.approval.state === "required";

  return (
    <div className="flex flex-col items-end gap-2">
      <div className="flex flex-wrap items-start justify-end gap-2">
        <ActionButton
          label="Record revision"
          cli={changeCli.revise(change.id)}
          unmet={ended}
          pending={run.isPending}
          onRun={() => run.mutate((id) => api.revise(id))}
        />
        <ActionButton
          label="Request changes"
          cli={changeCli.requestChanges(change.id)}
          unmet={ended}
          pending={run.isPending}
          onRun={() => run.mutate((id) => api.requestChanges(id))}
        />
        <ActionButton
          label="Approve"
          cli={changeCli.approve(change.id)}
          unmet={ended ?? (approved ? `Revision ${latest} is already approved` : null)}
          variant={needsApproval ? "default" : "outline"}
          pending={run.isPending}
          onRun={() => run.mutate((id) => api.approve(id, latest))}
        />
        <ActionButton
          label="Merge"
          cli={changeCli.merge(change.id)}
          unmet={mergeBlocker(change)}
          variant="default"
          pending={run.isPending}
          onRun={() => run.mutate((id) => api.merge(id))}
        />
        <ActionButton
          label="Close"
          cli={changeCli.close(change.id)}
          unmet={ended}
          pending={run.isPending}
          onRun={() => run.mutate((id) => api.close(id))}
        />
      </div>
      {run.isError ? (
        <p role="alert" className="max-w-md text-right text-sm text-failed">
          {run.error.message}
        </p>
      ) : null}
    </div>
  );
}
