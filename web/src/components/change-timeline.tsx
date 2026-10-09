import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { Change } from "@/api/client";
import { Empty } from "@/components/page";
import { format } from "@/lib/format";
import { queries } from "@/lib/queries";

type Entry = { at: string; text: string; link?: { run: string } };

/** The Timeline tab: revisions, runs, comments and approvals in the order they happened. */
export function TimelineTab({ change }: { change: Change }) {
  const runs = useQuery(queries.changeRuns(change.id));
  const entries: Entry[] = [
    ...change.revisions.map((revision) => ({
      at: revision.created_at,
      text:
        revision.number === 1
          ? `Opened as revision 1 on ${change.source_branch}`
          : `Revision ${revision.number} recorded`,
    })),
    ...(runs.data ?? []).map((run) => ({
      at: run.started_at,
      text: `Checks of revision ${run.revision} started`,
      link: { run: run.id },
    })),
    ...(change.comments ?? []).map((comment) => ({
      at: comment.at,
      text: `${comment.author} commented on revision ${comment.revision}${comment.path ? ` (${comment.path}${comment.line ? `:${comment.line}` : ""})` : ""}`,
    })),
    ...change.approvals.map((approval) => ({
      at: approval.at,
      text: `${approval.by} approved revision ${approval.revision}`,
    })),
  ].sort((a, b) => a.at.localeCompare(b.at));

  if (entries.length === 0) return <Empty>Nothing has happened yet.</Empty>;
  return (
    <ol aria-label="Timeline" className="flex max-w-3xl flex-col gap-2 text-sm">
      {entries.map((entry) => (
        <li key={`${entry.at}-${entry.text}`} className="flex gap-3">
          <time className="tabular w-36 shrink-0 text-xs text-muted-foreground">
            {format.time(entry.at)}
          </time>
          <span>{entry.text}</span>
          {entry.link ? (
            <Link
              to="/runs/$id"
              params={{ id: entry.link.run }}
              className="text-xs hover:underline"
            >
              Run
            </Link>
          ) : null}
        </li>
      ))}
      {change.phase === "merged" ? (
        <li className="flex gap-3">
          <span className="w-36 shrink-0" />
          <span>
            Merged{change.merged_commit ? ` as ${format.shortId(change.merged_commit)}` : ""}
          </span>
        </li>
      ) : null}
      {change.phase === "closed" ? (
        <li className="flex gap-3">
          <span className="w-36 shrink-0" />
          <span>Closed without merging</span>
        </li>
      ) : null}
    </ol>
  );
}
