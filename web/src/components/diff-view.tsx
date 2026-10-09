import { PatchDiff } from "@pierre/diffs/react";
import { useQuery } from "@tanstack/react-query";
import { ChevronRight } from "lucide-react";
import { useState } from "react";
import type { Change, Comment, FileDiff } from "@/api/client";
import { CommentCard, CommentForm } from "@/components/change-comments";
import { Await, Empty } from "@/components/page";
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
 * The Diff tab: the files of one revision, each as its unified patch on a dark surface, with the
 * revision's line comments under their lines. Hovering a line offers a comment on it.
 */
export default function DiffView({ change, revision }: { change: Change; revision: number }) {
  const diff = useQuery(queries.diff(change.id, revision));
  return (
    <Await query={diff} what="the diff">
      {(diff) =>
        diff.files.length === 0 ? (
          <Empty>Revision {diff.revision} changes no files.</Empty>
        ) : (
          <div className="flex flex-col gap-4">
            {diff.files.map((file) => (
              <FileView key={file.path} change={change} revision={diff.revision} file={file} />
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
}: {
  change: Change;
  revision: number;
  file: FileDiff;
}) {
  const [open, setOpen] = useState(true);
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
    <section aria-label={file.path} className="overflow-hidden rounded-lg border">
      <button
        type="button"
        aria-expanded={open}
        className="flex w-full items-center gap-2 bg-code px-3 py-2 text-left text-code-foreground"
        onClick={() => setOpen(!open)}
      >
        <ChevronRight className={cn("size-4 shrink-0 transition-transform", open && "rotate-90")} />
        <span className="min-w-0 flex-1 truncate font-mono text-sm">
          {file.previous_path ? `${file.previous_path} → ` : ""}
          {file.path}
        </span>
        <span className="text-xs text-code-muted">{STATUS_LABELS[file.status]}</span>
        <span className="tabular text-xs text-[#9be3b6]">+{file.additions}</span>
        <span className="tabular text-xs text-[#ffb3ba]">−{file.deletions}</span>
      </button>
      {open ? (
        <div className="bg-code text-code-foreground">
          {file.binary ? (
            <p className="px-4 py-3 text-sm text-code-muted">Binary file not shown.</p>
          ) : file.truncated || !file.patch ? (
            <p className="px-4 py-3 text-sm text-code-muted">
              {file.truncated
                ? "This file's patch is over 256 KiB and is not shown."
                : "No textual changes."}
            </p>
          ) : (
            <>
              {hint ? (
                <p role="status" className="px-4 pt-2 text-xs text-code-muted">
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
                    className="size-5 rounded bg-expedition text-xs leading-5 font-bold text-expedition-foreground"
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
                    <div className="m-2 text-foreground">
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
