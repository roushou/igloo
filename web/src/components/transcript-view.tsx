import { ChevronRight } from "lucide-react";
import { useMemo, useState } from "react";
import type { Task, Transcript } from "@/api/client";
import { VirtualList } from "@/components/virtual-list";
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
 * A task's transcript: its turns, what the tool said, and its tool calls collapsed to one line
 * each that open to the call's input, edits as diffs, and output on a dark surface.
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
      <p className="text-sm text-muted-foreground">
        {transcript.idle ? "The tool printed nothing." : "Waiting for the tool to start…"}
      </p>
    );
  }
  return (
    <VirtualList
      label="Transcript"
      className="max-h-[70vh] rounded-lg border bg-card"
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
        <div className="border-y bg-muted px-4 py-2 text-sm first:border-t-0">
          <span className="tabular font-medium">Turn {row.number}</span>
          {row.prompt ? (
            <p className="mt-0.5 line-clamp-3 whitespace-pre-wrap text-muted-foreground">
              {row.prompt}
            </p>
          ) : null}
          {row.reason ? <p className="mt-0.5 text-errored">{row.reason}</p> : null}
        </div>
      );
    case "message":
      return <p className="px-4 py-2 text-sm whitespace-pre-wrap">{row.text}</p>;
    case "output":
      return (
        <p className="px-4 py-0.5 font-mono text-xs whitespace-pre-wrap text-muted-foreground">
          {row.text}
        </p>
      );
    case "error":
      return <p className="px-4 py-2 text-sm whitespace-pre-wrap text-failed">{row.text}</p>;
    case "tool":
      return <ToolCall row={row} />;
  }
}

function ToolCall({ row }: { row: ToolRow }) {
  const [open, setOpen] = useState(false);
  const edits = useMemo(() => toolEdits(row.input), [row.input]);
  const failed = row.result?.isError === true;
  return (
    <div className="px-4 py-1">
      <button
        type="button"
        aria-expanded={open}
        className="flex w-full items-center gap-2 rounded px-1 py-1 text-left text-sm hover:bg-accent"
        onClick={() => setOpen(!open)}
      >
        <ChevronRight
          className={cn("size-3.5 shrink-0 transition-transform", open && "rotate-90")}
        />
        <span className="font-mono text-xs font-medium">{row.name}</span>
        <span className="min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground">
          {toolSummary(row.input)}
        </span>
        {failed ? <span className="text-xs text-failed">failed</span> : null}
        {row.result === null ? <span className="text-xs text-running">running</span> : null}
      </button>
      {open ? (
        <div className="mt-1 mb-2 flex flex-col gap-2">
          {edits.length > 0 ? (
            edits.map((edit) => <EditDiff key={edit.key} edit={edit} />)
          ) : row.input ? (
            <Dark>{row.input}</Dark>
          ) : null}
          {row.result ? (
            <Dark tone={failed ? "error" : undefined}>{row.result.output || "(no output)"}</Dark>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function Dark({ children, tone }: { children: string; tone?: "error" }) {
  return (
    <pre
      className={cn(
        "max-h-80 overflow-auto rounded-md bg-code px-3 py-2 text-xs whitespace-pre-wrap text-code-foreground",
        tone === "error" && "text-[#ff9aa2]",
      )}
    >
      {children}
    </pre>
  );
}

function EditDiff({ edit }: { edit: Edit }) {
  const prefixed = (sign: string, text: string) =>
    text
      .split("\n")
      .map((line) => `${sign} ${line}`)
      .join("\n");
  return (
    <div className="overflow-hidden rounded-md bg-code text-xs text-code-foreground">
      {edit.path ? (
        <p className="border-b border-code-line px-3 py-1 text-code-muted">{edit.path}</p>
      ) : null}
      <pre className="max-h-80 overflow-auto py-1 whitespace-pre-wrap">
        {edit.removed ? (
          <span className="block bg-[#3a1a20] px-3 text-[#ffb3ba]">
            {prefixed("-", edit.removed)}
          </span>
        ) : null}
        <span className="block bg-[#12301f] px-3 text-[#9be3b6]">{prefixed("+", edit.added)}</span>
      </pre>
    </div>
  );
}
