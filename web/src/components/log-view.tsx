import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { api } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { type ListHandle, VirtualList } from "@/components/virtual-list";
import { firstErrorLine, JobLog, type LogLine } from "@/lib/job-log";
import { cn } from "@/lib/utils";

const ROW_HEIGHT = 20;

/** Reads job `id`'s output into a store that lives as long as the component. */
function useJobLog(id: string) {
  const [log, setLog] = useState<JobLog | null>(null);
  useEffect(() => {
    const next = new JobLog((lastId, signal) => api.jobLogs(id, lastId, signal));
    setLog(next);
    next.start();
    return () => next.stop();
  }, [id]);
  const empty = useMemo(
    () => ({ version: 0, state: "loading" as const, lines: [] as LogLine[], job: null }),
    [],
  );
  return useSyncExternalStore(
    log?.subscribe ?? noSubscribe,
    log?.getSnapshot ?? (() => empty),
    () => empty,
  );
}

const noSubscribe = () => () => {};

const STATE_TEXT = {
  loading: "Connecting…",
  streaming: "Following",
  ended: "Finished",
  failed: "Disconnected, retrying…",
} as const;

/**
 * A job's whole output on a dark surface. It follows the job while it runs, searches the lines,
 * and jumps to the first line that mentions `error`. With `openAtError` it opens there once the
 * output has ended.
 */
export function LogView({
  jobId,
  openAtError,
  className,
}: {
  jobId: string;
  openAtError?: boolean;
  className?: string;
}) {
  const log = useJobLog(jobId);
  const list = useRef<ListHandle>(null);
  const [follow, setFollow] = useState(true);
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(-1);
  const [highlight, setHighlight] = useState<number | null>(null);
  const opened = useRef<string | null>(null);

  const matches = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return [];
    const found: number[] = [];
    log.lines.forEach((line, index) => {
      if (line.text.toLowerCase().includes(needle)) found.push(index);
    });
    return found;
  }, [log.lines, query]);

  const first = useMemo(() => firstErrorLine(log.lines), [log.lines]);

  const goTo = (index: number) => {
    setFollow(false);
    setHighlight(index);
    list.current?.scrollToIndex(index);
  };

  useEffect(() => {
    if (!openAtError || log.state !== "ended" || opened.current === jobId) return;
    opened.current = jobId;
    if (first >= 0) {
      setFollow(false);
      setHighlight(first);
      list.current?.scrollToIndex(first);
    }
  }, [openAtError, log.state, first, jobId]);

  const step = (by: number) => {
    if (matches.length === 0) return;
    const next =
      cursor < 0
        ? by > 0
          ? 0
          : matches.length - 1
        : (cursor + by + matches.length) % matches.length;
    setCursor(next);
    goTo(matches[next] as number);
  };

  return (
    <div className={cn("flex min-h-0 flex-col overflow-hidden rounded-lg border", className)}>
      <div className="flex flex-wrap items-center gap-2 border-b border-code-line bg-code px-3 py-2 text-code-foreground">
        <Input
          aria-label="Search the log"
          placeholder="Search"
          className="h-8 w-48 border-code-line bg-transparent text-code-foreground"
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setCursor(-1);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") step(event.shiftKey ? -1 : 1);
          }}
        />
        <span aria-live="polite" className="tabular text-xs text-code-muted">
          {query.trim() ? `${matches.length} match${matches.length === 1 ? "" : "es"}` : null}
        </span>
        <Button variant="ghost" size="sm" disabled={matches.length === 0} onClick={() => step(-1)}>
          Previous
        </Button>
        <Button variant="ghost" size="sm" disabled={matches.length === 0} onClick={() => step(1)}>
          Next
        </Button>
        <Button variant="ghost" size="sm" disabled={first < 0} onClick={() => goTo(first)}>
          Jump to first error
        </Button>
        <label className="ml-auto flex items-center gap-1.5 text-xs">
          <input
            type="checkbox"
            checked={follow}
            onChange={(event) => setFollow(event.target.checked)}
          />
          Follow
        </label>
        <span role="status" className="text-xs text-code-muted">
          {STATE_TEXT[log.state]}
        </span>
      </div>
      <VirtualList
        ref={list}
        label="Job output"
        className="min-h-0 flex-1 bg-code py-2 text-code-foreground"
        items={log.lines}
        rowHeight={ROW_HEIGHT}
        follow={follow && log.state !== "ended"}
        onLeaveEnd={() => setFollow(false)}
        itemKey={(_, index) => index}
        renderItem={(line, index) => (
          <LogRow
            line={line}
            number={index + 1}
            highlighted={index === highlight}
            needle={query.trim()}
          />
        )}
      />
      {log.lines.length === 0 ? (
        <p className="bg-code px-3 pb-3 text-sm text-code-muted">
          {log.state === "ended" ? "The job printed nothing." : "Waiting for output…"}
        </p>
      ) : null}
    </div>
  );
}

function LogRow({
  line,
  number,
  highlighted,
  needle,
}: {
  line: LogLine;
  number: number;
  highlighted: boolean;
  needle: string;
}) {
  return (
    <div
      data-highlighted={highlighted || undefined}
      className={cn(
        "flex h-5 gap-3 px-3 font-mono text-[13px] leading-5 whitespace-pre",
        line.stream === "stderr" && "text-[#ff9aa2]",
        highlighted && "bg-[#2a3a48]",
      )}
    >
      <span aria-hidden className="tabular w-10 shrink-0 text-right text-code-muted select-none">
        {number}
      </span>
      <span>{needle ? <Marked text={line.text} needle={needle} /> : line.text}</span>
    </div>
  );
}

function Marked({ text, needle }: { text: string; needle: string }) {
  const lower = text.toLowerCase();
  const target = needle.toLowerCase();
  const parts: React.ReactNode[] = [];
  let from = 0;
  for (let at = lower.indexOf(target); at !== -1; at = lower.indexOf(target, from)) {
    if (at > from) parts.push(text.slice(from, at));
    parts.push(
      <mark key={at} className="rounded-sm bg-[#e0b13a] text-black">
        {text.slice(at, at + needle.length)}
      </mark>,
    );
    from = at + needle.length;
  }
  parts.push(text.slice(from));
  return <>{parts}</>;
}
