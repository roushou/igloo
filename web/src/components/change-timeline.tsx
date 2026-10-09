import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import {
  Check,
  GitCommitHorizontal,
  GitMerge,
  type LucideIcon,
  MessageSquare,
  Play,
  X,
} from "lucide-react";
import type { Change } from "@/api/client";
import { Empty } from "@/components/page";
import { RelativeTime } from "@/components/relative-time";
import { format } from "@/lib/format";
import { queries } from "@/lib/queries";
import { timing } from "@/lib/timing";
import { cn } from "@/lib/utils";

type Entry = {
  at: string | null;
  text: string;
  icon: LucideIcon;
  tone?: "passed" | "closed";
  link?: { run: string };
};

/**
 * The Timeline tab: revisions, runs, comments and approvals in the order they happened, ending
 * with the merge or the close. The end shows its time once the server records one.
 */
export function TimelineTab({ change }: { change: Change }) {
  const runs = useQuery(queries.changeRuns(change.id));
  const entries: (Entry & { at: string })[] = [
    ...change.revisions.map((revision) => ({
      at: revision.created_at,
      icon: GitCommitHorizontal,
      text:
        revision.number === 1
          ? `Opened as revision 1 on ${change.source_branch}`
          : `Revision ${revision.number} recorded`,
    })),
    ...(runs.data ?? []).map((run) => ({
      at: run.started_at,
      icon: Play,
      text: `Checks of revision ${run.revision} started`,
      link: { run: run.id },
    })),
    ...(change.comments ?? []).map((comment) => ({
      at: comment.at,
      icon: MessageSquare,
      text: `${comment.author} commented on revision ${comment.revision}${comment.path ? ` (${comment.path}${comment.line ? `:${comment.line}` : ""})` : ""}`,
    })),
    ...change.approvals.map((approval) => ({
      at: approval.at,
      icon: Check,
      text: `${approval.by} approved revision ${approval.revision}`,
    })),
  ].sort((a, b) => a.at.localeCompare(b.at));

  const endedAt = timing.changeEnd(change);
  const end: Entry | null =
    change.phase === "merged"
      ? {
          at: endedAt,
          icon: GitMerge,
          tone: "passed",
          text: `Merged${change.merged_commit ? ` as ${format.shortId(change.merged_commit)}` : ""}`,
        }
      : change.phase === "closed"
        ? { at: endedAt, icon: X, tone: "closed", text: "Closed without merging" }
        : null;

  if (entries.length === 0) return <Empty>Nothing has happened yet.</Empty>;
  return (
    <ol aria-label="Timeline" className="flex max-w-3xl flex-col text-base">
      {[...entries, ...(end ? [end] : [])].map((entry, index, all) => (
        <li key={`${entry.at}-${entry.text}`} className="relative flex gap-3 pb-4 last:pb-0">
          {index < all.length - 1 ? (
            <span aria-hidden className="absolute top-6 bottom-0 left-[11px] w-px bg-border" />
          ) : null}
          <span
            aria-hidden
            className={cn(
              "z-10 flex size-6 shrink-0 items-center justify-center rounded-full border bg-card text-muted-foreground",
              entry.tone === "passed" && "border-passed/40 bg-passed-soft text-passed",
              entry.tone === "closed" && "border-closed/40 bg-closed-soft text-closed",
            )}
          >
            <entry.icon className="size-3.5" />
          </span>
          <div className="flex min-w-0 flex-1 flex-wrap items-baseline gap-x-3 pt-0.5">
            <span className={cn(entry.tone && "font-medium")}>{entry.text}</span>
            {entry.link ? (
              <Link
                to="/runs/$id"
                params={{ id: entry.link.run }}
                className="text-sm text-muted-foreground hover:text-foreground hover:underline"
              >
                Run
              </Link>
            ) : null}
            {entry.at ? (
              <RelativeTime
                at={entry.at}
                className="tabular ml-auto text-sm text-muted-foreground"
              />
            ) : null}
          </div>
        </li>
      ))}
    </ol>
  );
}
