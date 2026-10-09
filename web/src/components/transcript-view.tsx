import { ChevronRight, Loader, TriangleAlert } from "lucide-react";
import { useMemo, useState } from "react";
import type { Task, Transcript } from "@/api/client";
import { CodeBlock } from "@/components/code-block";
import { VirtualList } from "@/components/virtual-list";
import { format } from "@/lib/format";
import {
  type Edit,
  type Row,
  type ToolRow,
  toolEdits,
  toolSummary,
  transcriptRows,
} from "@/lib/transcript";
import { cn } from "@/lib/utils";

/**
 * A task's transcript on the dark surface: its turns, what the tool said, and its tool calls
 * collapsed to one line each that open to the call's input, edits as diffs, and output.
 */
export function TranscriptView({
  transcript,
  turns,
  follow,
  onLeaveEnd,
}: {
  transcript: Transcript;
  turns: Task["turns"];
  follow: boolean;
  onLeaveEnd: () => void;
}) {
  const rows = useMemo(
    () => transcriptRows(transcript.entries, turns),
    [transcript.entries, turns],
  );
  if (rows.length === 0) {
    return (
      <div className="flex items-center gap-2 rounded-lg bg-code px-4 py-6 text-base text-code-muted">
        {transcript.idle ? null : <Loader className="size-4 animate-spin" />}
        {transcript.idle ? "The tool printed nothing." : "Waiting for the tool to start…"}
      </div>
    );
  }
  return (
    <VirtualList
      label="Transcript"
      className="max-h-[min(70vh,52rem)] rounded-lg bg-code text-code-foreground ring-1 ring-code-line"
      items={rows}
      rowHeight={44}
      measure
      follow={follow}
      onLeaveEnd={onLeaveEnd}
      itemKey={(row) => row.key}
      renderItem={(row) => <TranscriptRow row={row} />}
    />
  );
}

function TranscriptRow({ row }: { row: Row }) {
  switch (row.kind) {
    case "turn":
      return (
        <div className="flex flex-col gap-0.5 border-y border-code-line bg-code-raised px-4 py-2 first:border-t-0">
          <span className="tabular text-sm font-medium tracking-wide text-code-muted uppercase">
            Turn {row.number}
          </span>
          {row.prompt ? (
            <p className="line-clamp-3 text-base whitespace-pre-wrap text-code-foreground/80">
              {row.prompt}
            </p>
          ) : null}
          {row.reason ? <p className="text-base text-code-error">{row.reason}</p> : null}
        </div>
      );
    case "message":
      return (
        <p className="px-4 py-2.5 text-base leading-6 whitespace-pre-wrap text-code-foreground">
          {row.text}
        </p>
      );
    case "output":
      return (
        <p className="px-4 py-0.5 font-mono text-sm whitespace-pre-wrap text-code-muted">
          {row.text}
        </p>
      );
    case "error":
      return (
        <p className="flex gap-2 px-4 py-2 text-base whitespace-pre-wrap text-code-error">
          <TriangleAlert className="mt-0.5 size-4 shrink-0" />
          {row.text}
        </p>
      );
    case "tool":
      return <ToolCall row={row} />;
  }
}

function ToolCall({ row }: { row: ToolRow }) {
  const [open, setOpen] = useState(false);
  const edits = useMemo(() => toolEdits(row.input), [row.input]);
  const failed = row.result?.isError === true;
  const lines = row.result ? row.result.output.split("\n").filter(Boolean).length : 0;
  return (
    <div className="px-2 py-0.5">
      <button
        type="button"
        aria-expanded={open}
        className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left hover:bg-code-raised"
        onClick={() => setOpen(!open)}
      >
        <ChevronRight
          className={cn(
            "size-3.5 shrink-0 text-code-muted transition-transform",
            open && "rotate-90",
          )}
        />
        <span className="shrink-0 rounded bg-code-line px-1.5 py-px font-mono text-sm font-medium">
          {row.name}
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-sm text-code-muted">
          {toolSummary(row.input)}
        </span>
        <ToolStatus
          failed={failed}
          running={row.result === null}
          lines={lines}
          durationMs={row.durationMs}
        />
      </button>
      {open ? (
        <div className="mt-1 mb-2 ml-7 flex flex-col gap-2">
          {edits.length > 0 ? (
            edits.map((edit) => <EditDiff key={edit.key} edit={edit} />)
          ) : row.input ? (
            <CodeBlock text={row.input} />
          ) : null}
          {row.result ? (
            <CodeBlock
              text={row.result.output || "(no output)"}
              tone={failed ? "error" : undefined}
            />
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/** What the right end of a tool call says: running, failed, how long it took and how much it printed. */
function ToolStatus({
  failed,
  running,
  lines,
  durationMs,
}: {
  failed: boolean;
  running: boolean;
  lines: number;
  durationMs: number | null;
}) {
  if (running) {
    return (
      <span className="flex shrink-0 items-center gap-1.5 text-sm text-running">
        <span aria-hidden className="size-1.5 animate-pulse-dot rounded-full bg-current" />
        running
      </span>
    );
  }
  return (
    <span className="flex shrink-0 items-center gap-2 text-sm text-code-muted">
      {failed ? <span className="text-code-error">failed</span> : null}
      {durationMs === null ? null : <span className="tabular">{format.duration(durationMs)}</span>}
      {lines > 0 ? (
        <span className="tabular">
          {lines} {lines === 1 ? "line" : "lines"}
        </span>
      ) : null}
    </span>
  );
}

function EditDiff({ edit }: { edit: Edit }) {
  const prefixed = (sign: string, text: string) =>
    text
      .split("\n")
      .map((line) => `${sign} ${line}`)
      .join("\n");
  const text = `${edit.removed ? `${prefixed("-", edit.removed)}\n` : ""}${prefixed("+", edit.added)}`;
  return <CodeBlock diff text={text} label={edit.path || undefined} />;
}
