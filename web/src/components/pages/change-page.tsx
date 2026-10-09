import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowRight,
  Check,
  CircleCheck,
  CircleX,
  Clock,
  Ellipsis,
  GitMerge,
  MessageSquareWarning,
  RotateCw,
  X,
} from "lucide-react";
import { motion } from "motion/react";
import { lazy, Suspense, useMemo, useState } from "react";
import { api, type Change } from "@/api/client";
import { ActionButton, CliMenu } from "@/components/action-button";
import { ChecksTab } from "@/components/change-checks";
import { CommentsTab } from "@/components/change-comments";
import { TimelineTab } from "@/components/change-timeline";
import { Await, Fact, Loading, Page } from "@/components/page";
import { RelativeTime } from "@/components/relative-time";
import { ShortId } from "@/components/short-id";
import { StatusPill } from "@/components/status-pill";
import { Button } from "@/components/ui/button";
import { Modal, ModalBody } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useToast } from "@/components/ui/toast";
import { Tooltip } from "@/components/ui/tooltip";
import { changeCli } from "@/lib/cli";
import { type Command, useRegisterCommands } from "@/lib/commands";
import { format } from "@/lib/format";
import { useMotion } from "@/lib/motion";
import { queries, queryKeys } from "@/lib/queries";
import { mergeBlocker, mergeRules, type Rule } from "@/lib/readiness";
import { type State, status } from "@/lib/status";
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

type ActionName = "revise" | "request-changes" | "approve" | "merge" | "close";

function ChangeDetail({ change }: { change: Change }) {
  const latest = change.revisions.at(-1)?.number ?? 1;
  const [picked, setPicked] = useState<number | null>(null);
  const revision = picked ?? latest;
  const [tab, setTab] = useState<Tab>("Diff");
  const [confirming, setConfirming] = useState<"merge" | "close" | null>(null);
  const state = status.change(change);
  const rules = mergeRules(change);
  const actions = useActions(change);

  const open = change.phase === "open";
  const ended = open ? null : `The change is already ${change.phase}`;
  const approved =
    change.readiness?.approval.state === "given" ||
    change.approvals.some((approval) => approval.revision === latest);
  const needsApproval = change.readiness?.approval.state === "required";
  const revise = ended;
  const requestChanges = ended;
  const approve = ended ?? (approved ? `Revision ${latest} is already approved` : null);
  const merge = mergeBlocker(change);
  const close = ended;
  const unmet = { revise, "request-changes": requestChanges, approve, merge, close };

  const commands = useMemo<Command[]>(
    () => [
      {
        id: "merge",
        label: "Merge this change",
        group: "This change",
        icon: GitMerge,
        shortcut: undefined,
        unmet: merge,
        run: () => setConfirming("merge"),
      },
      {
        id: "approve",
        label: "Approve this change",
        group: "This change",
        icon: Check,
        unmet: approve,
        run: () => actions.run("approve"),
      },
      {
        id: "request-changes",
        label: "Request changes",
        group: "This change",
        icon: MessageSquareWarning,
        unmet: requestChanges,
        run: () => actions.run("request-changes"),
      },
      {
        id: "revise",
        label: "Record a revision",
        group: "This change",
        icon: RotateCw,
        unmet: revise,
        run: () => actions.run("revise"),
      },
      {
        id: "close",
        label: "Close this change",
        group: "This change",
        icon: X,
        unmet: close,
        run: () => setConfirming("close"),
      },
    ],
    [merge, approve, requestChanges, revise, close, actions.run],
  );
  useRegisterCommands(commands);

  const confirm = (name: "merge" | "close") => {
    setConfirming(null);
    actions.run(name);
  };

  return (
    <Page
      wide
      crumbs={[
        { label: "Changes", link: { to: "/changes", search: true } },
        { label: <span className="font-mono text-sm">{format.shortId(change.id)}</span> },
      ]}
      title={change.title}
      meta={
        <>
          <StatusPill state={state} />
          <span className="inline-flex items-center gap-1.5 font-mono text-sm">
            {change.source_branch}
            <ArrowRight className="size-3.5 text-muted-foreground" />
            {change.target_branch}
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
            <span>Revision</span>
            <select
              aria-label="Revision"
              className="h-7 rounded-md border bg-card px-1.5 text-base text-foreground"
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
          {change.revisions[0] ? (
            <Fact label="Opened">
              <RelativeTime at={change.revisions[0].created_at} />
            </Fact>
          ) : null}
        </>
      }
      actions={
        <>
          <CliMenu
            commands={[
              { label: "Record revision", command: changeCli.revise(change.id) },
              { label: "Request changes", command: changeCli.requestChanges(change.id) },
              { label: "Approve", command: changeCli.approve(change.id) },
              { label: "Merge", command: changeCli.merge(change.id) },
              { label: "Close", command: changeCli.close(change.id) },
            ]}
          />
          <DropdownMenu>
            <Tooltip content="More actions">
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="icon" aria-label="More actions">
                  <Ellipsis />
                </Button>
              </DropdownMenuTrigger>
            </Tooltip>
            <DropdownMenuContent>
              <MenuAction
                label="Record revision"
                icon={<RotateCw />}
                unmet={unmet.revise}
                onRun={() => actions.run("revise")}
              />
              <MenuAction
                label="Request changes"
                icon={<MessageSquareWarning />}
                unmet={unmet["request-changes"]}
                onRun={() => actions.run("request-changes")}
              />
              <MenuAction
                label="Close"
                icon={<X />}
                unmet={unmet.close}
                tone="danger"
                onRun={() => setConfirming("close")}
              />
            </DropdownMenuContent>
          </DropdownMenu>
          <ActionButton
            label="Approve"
            variant={needsApproval && !approved ? "expedition" : "outline"}
            unmet={unmet.approve}
            pending={actions.pending === "approve"}
            onRun={() => actions.run("approve")}
          />
          <ActionButton
            label="Merge"
            variant={state === "needs-you" && !unmet.merge ? "expedition" : "default"}
            unmet={unmet.merge}
            pending={actions.pending === "merge"}
            onRun={() => setConfirming("merge")}
          />
        </>
      }
    >
      {actions.error ? (
        <p
          role="alert"
          className="rounded-lg border border-failed/30 bg-failed-soft px-4 py-3 text-base text-failed"
        >
          {actions.error}
        </p>
      ) : null}
      {rules.length > 0 ? <Checklist rules={rules} /> : null}

      <Tabs tab={tab} onTab={setTab} counts={{ Comments: change.comments?.length ?? 0 }} />
      <div role="tabpanel" aria-label={tab}>
        {tab === "Diff" ? (
          <Suspense fallback={<Loading what="the diff" rows={3} />}>
            <DiffView change={change} revision={revision} />
          </Suspense>
        ) : null}
        {tab === "Checks" ? <ChecksTab changeId={change.id} revision={revision} /> : null}
        {tab === "Comments" ? <CommentsTab change={change} revision={revision} /> : null}
        {tab === "Timeline" ? <TimelineTab change={change} /> : null}
      </div>

      <Modal
        open={confirming !== null}
        onOpenChange={(next) => !next && setConfirming(null)}
        label={confirming === "merge" ? "Merge this change" : "Close this change"}
        role="alertdialog"
      >
        {confirming === "merge" ? (
          <ModalBody
            title="Merge this change?"
            description={
              <>
                The latest revision of{" "}
                <span className="font-medium text-foreground">{change.title}</span> is merged into{" "}
                {change.target_branch}.
              </>
            }
          >
            <Button variant="ghost" onClick={() => setConfirming(null)}>
              Cancel
            </Button>
            <Button variant="expedition" onClick={() => confirm("merge")}>
              <GitMerge />
              Merge
            </Button>
          </ModalBody>
        ) : (
          <ModalBody
            title="Close this change?"
            description="It is closed without merging, and takes no more revisions, comments or approvals."
          >
            <Button variant="ghost" onClick={() => setConfirming(null)}>
              Keep open
            </Button>
            <Button variant="danger" onClick={() => confirm("close")}>
              Close change
            </Button>
          </ModalBody>
        )}
      </Modal>
    </Page>
  );
}

function MenuAction({
  label,
  icon,
  unmet,
  tone,
  onRun,
}: {
  label: string;
  icon: React.ReactNode;
  unmet: string | null;
  tone?: "danger";
  onRun: () => void;
}) {
  return (
    <DropdownMenuItem disabled={Boolean(unmet)} tone={tone} onSelect={onRun}>
      {icon}
      <span className="flex-1">
        {label}
        {unmet ? <span className="block text-sm text-muted-foreground">{unmet}</span> : null}
      </span>
    </DropdownMenuItem>
  );
}

const RULE_ICONS: Record<State, typeof Check> = {
  passed: CircleCheck,
  failed: CircleX,
  running: Clock,
  "needs-you": MessageSquareWarning,
  errored: CircleX,
  closed: Clock,
};

const RULE_COLORS: Record<State, string> = {
  passed: "text-passed",
  failed: "text-failed",
  running: "text-running",
  "needs-you": "text-expedition",
  errored: "text-errored",
  closed: "text-muted-foreground",
};

/** The merge rules, one cell each: met in green, waiting on the user in orange. */
function Checklist({ rules }: { rules: Rule[] }) {
  return (
    <ul
      aria-label="Merge checklist"
      className="grid gap-px overflow-hidden rounded-lg border bg-border md:grid-cols-3"
    >
      {rules.map((rule) => {
        const Icon = RULE_ICONS[rule.state];
        return (
          <li
            key={rule.id}
            data-state={rule.state}
            data-met={rule.met}
            className="flex items-start gap-2.5 bg-card px-4 py-3 text-base"
          >
            <Icon className={cn("mt-0.5 size-4 shrink-0", RULE_COLORS[rule.state])} />
            <span
              className={cn(
                !rule.met && rule.state === "needs-you" && "font-medium text-expedition",
              )}
            >
              {rule.text}
            </span>
          </li>
        );
      })}
    </ul>
  );
}

/** The tab strip: a sliding underline marks the open tab. It sticks under the page header. */
function Tabs({
  tab,
  onTab,
  counts,
}: {
  tab: Tab;
  onTab: (tab: Tab) => void;
  counts: Partial<Record<Tab, number>>;
}) {
  const { transition } = useMotion();
  return (
    <div
      role="tablist"
      aria-label="Change"
      className="sticky top-0 z-20 -mx-4 -mb-2 flex gap-1 border-b bg-card/90 px-4 backdrop-blur sm:-mx-8 sm:px-8"
    >
      {TABS.map((name) => (
        <button
          key={name}
          type="button"
          role="tab"
          aria-selected={tab === name}
          className={cn(
            "relative flex h-10 items-center gap-1.5 px-3 text-base text-muted-foreground transition-colors hover:text-foreground",
            tab === name && "font-medium text-foreground",
          )}
          onClick={() => onTab(name)}
        >
          {name}
          {(counts[name] ?? 0) > 0 ? (
            <span className="tabular rounded-full bg-muted px-1.5 text-xs font-medium">
              {counts[name]}
            </span>
          ) : null}
          {tab === name ? (
            <motion.span
              layoutId="change-tab"
              transition={transition(0.2)}
              className="absolute inset-x-2 -bottom-px h-0.5 rounded-full bg-foreground"
            />
          ) : null}
        </button>
      ))}
    </div>
  );
}

/** Runs the actions on a change, keeps the result as its data, and tells the user what happened. */
function useActions(change: Change) {
  const queryClient = useQueryClient();
  const toast = useToast();
  const [error, setError] = useState<string | null>(null);
  const latest = change.revisions.at(-1)?.number;
  const call: Record<ActionName, [() => Promise<Change>, string, string?]> = {
    revise: [() => api.revise(change.id), "Revision recorded"],
    "request-changes": [
      () => api.requestChanges(change.id),
      "Changes requested",
      "The agent receives your comments.",
    ],
    approve: [() => api.approve(change.id, latest), `Revision ${latest} approved`],
    merge: [() => api.merge(change.id), "Change merged", `Into ${change.target_branch}.`],
    close: [() => api.close(change.id), "Change closed"],
  };
  const mutation = useMutation({
    mutationFn: (name: ActionName) => call[name][0](),
    onMutate: () => setError(null),
    onSuccess: async (updated, name) => {
      queryClient.setQueryData(queryKeys.change(change.id), updated);
      await queryClient.invalidateQueries({ queryKey: queryKeys.changes(change.repo) });
      await queryClient.invalidateQueries({ queryKey: queryKeys.changeRuns(change.id) });
      toast.show({ title: call[name][1], description: call[name][2] });
    },
    onError: (failure) => setError(failure.message),
  });
  const { mutate, isPending, variables } = mutation;
  return {
    run: mutate,
    pending: isPending ? variables : null,
    error,
  };
}
