import { PatchDiff } from "@pierre/diffs/react";
import { useQuery } from "@tanstack/react-query";
import { ChevronRight, FileDiff as FileIcon } from "lucide-react";
import { useState } from "react";
import type { Change, Comment, FileDiff } from "@/api/client";
import { CommentCard, CommentForm } from "@/components/change-comments";
import { EmptyState } from "@/components/empty-state";
import { Await } from "@/components/page";
import { Button } from "@/components/ui/button";
import { queries } from "@/lib/queries";
import { cn } from "@/lib/utils";

/** What an annotation on a diff line holds: a comment, or `null` for the box that writes one. */
type Note = { comment: Comment | null };

const STATUS_LABELS: Record<FileDiff["status"], string> = {
  added: "added",
  modified: "modified",
  deleted: "deleted",
  renamed: "renamed",
};

/**
 * The Diff tab: the files of one revision, each as its unified patch on a dark surface under a
 * header that sticks while the file scrolls, with the revision's line comments under their
 * lines. Hovering a line offers a comment on it.
 */
export default function DiffView({ change, revision }: { change: Change; revision: number }) {
  const diff = useQuery(queries.diff(change.id, revision));
  const [closed, setClosed] = useState<ReadonlySet<string>>(new Set());
  return (
    <Await query={diff} what="the diff" rows={3}>
      {(diff) =>
        diff.files.length === 0 ? (
          <EmptyState icon={FileIcon} title={`Revision ${diff.revision} changes no files.`}>
            Nothing differs from {change.target_branch} at this revision.
          </EmptyState>
        ) : (
          <div className="flex flex-col gap-4">
            <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-base text-muted-foreground">
              <span>
                <span className="tabular font-medium text-foreground">{diff.files.length}</span>{" "}
                {diff.files.length === 1 ? "file" : "files"} changed
              </span>
              <span className="tabular text-passed">
                +{diff.files.reduce((sum, file) => sum + file.additions, 0)}
              </span>
              <span className="tabular text-failed">
                −{diff.files.reduce((sum, file) => sum + file.deletions, 0)}
              </span>
              <span className="ml-auto flex gap-1">
                <Button variant="ghost" size="sm" onClick={() => setClosed(new Set())}>
                  Expand all
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setClosed(new Set(diff.files.map((file) => file.path)))}
                >
                  Collapse all
                </Button>
              </span>
            </div>
            {diff.files.map((file) => (
              <FileView
                key={file.path}
                change={change}
                revision={diff.revision}
                file={file}
                open={!closed.has(file.path)}
                onToggle={() =>
                  setClosed((all) => {
                    const next = new Set(all);
                    if (!next.delete(file.path)) next.add(file.path);
                    return next;
                  })
                }
              />
            ))}
          </div>
        )
      }
    </Await>
  );
}

function FileView({
  change,
  revision,
  file,
  open,
  onToggle,
}: {
  change: Change;
  revision: number;
  file: FileDiff;
  open: boolean;
  onToggle: () => void;
}) {
  const [draft, setDraft] = useState<number | null>(null);
  const [hint, setHint] = useState<string | null>(null);

  const notes: { side: "additions"; lineNumber: number; metadata: Note }[] = [
    ...(change.comments ?? [])
      .filter(
        (comment) =>
          comment.path === file.path && comment.line != null && comment.revision === revision,
      )
      .map((comment) => ({
        side: "additions" as const,
        lineNumber: comment.line as number,
        metadata: { comment },
      })),
    ...(draft === null
      ? []
      : [{ side: "additions" as const, lineNumber: draft, metadata: { comment: null } }]),
  ];

  return (
    <section
      aria-label={file.path}
      className="overflow-clip rounded-lg bg-code text-code-foreground ring-1 ring-code-line"
    >
      <button
        type="button"
        aria-expanded={open}
        className="sticky top-10 z-10 flex w-full items-center gap-2 border-b border-code-line bg-code-raised px-3 py-2 text-left"
        onClick={onToggle}
      >
        <ChevronRight
          className={cn(
            "size-4 shrink-0 text-code-muted transition-transform",
            open && "rotate-90",
          )}
        />
        <span className="min-w-0 flex-1 truncate font-mono text-base">
          {file.previous_path ? `${file.previous_path} → ` : ""}
          {file.path}
        </span>
        <span className="text-sm text-code-muted">{STATUS_LABELS[file.status]}</span>
        <span className="tabular text-sm text-code-added-foreground">+{file.additions}</span>
        <span className="tabular text-sm text-code-removed-foreground">−{file.deletions}</span>
      </button>
      {open ? (
        <div>
          {file.binary ? (
            <p className="px-4 py-3 text-base text-code-muted">Binary file not shown.</p>
          ) : file.truncated || !file.patch ? (
            <p className="px-4 py-3 text-base text-code-muted">
              {file.truncated
                ? "This file's patch is over 256 KiB and is not shown."
                : "No textual changes."}
            </p>
          ) : (
            <>
              {hint ? (
                <p role="status" className="px-4 pt-2 text-sm text-code-muted">
                  {hint}
                </p>
              ) : null}
              <PatchDiff<Note>
                patch={file.patch}
                disableWorkerPool
                options={{
                  theme: "pierre-dark",
                  themeType: "dark",
                  diffStyle: "unified",
                  disableFileHeader: true,
                  overflow: "scroll",
                  enableGutterUtility: true,
                }}
                lineAnnotations={notes}
                renderGutterUtility={(hovered) => (
                  <button
                    type="button"
                    aria-label="Comment on this line"
                    className="size-5 rounded bg-code-foreground text-sm leading-5 font-bold text-code"
                    onClick={() => {
                      const line = hovered();
                      if (line?.side === "additions") {
                        setHint(null);
                        setDraft(line.lineNumber);
                      } else {
                        setHint("Comments go on added or unchanged lines.");
                      }
                    }}
                  >
                    +
                  </button>
                )}
                renderAnnotation={(annotation) =>
                  annotation.metadata.comment ? (
                    <div className="m-2 rounded-lg border bg-card p-3 text-foreground">
                      <CommentCard comment={annotation.metadata.comment} />
                    </div>
                  ) : (
                    <div className="m-2 rounded-lg border bg-card p-3 text-foreground">
                      <CommentForm
                        change={change}
                        revision={revision}
                        where={{ path: file.path, line: annotation.lineNumber }}
                        label={`Comment on ${file.path} line ${annotation.lineNumber}`}
                        onDone={() => setDraft(null)}
                      />
                    </div>
                  )
                }
              />
            </>
          )}
        </div>
      ) : null}
    </section>
  );
}
