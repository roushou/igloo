import { ArrowDownToLine, ChevronDown, ChevronUp, Search, TriangleAlert } from "lucide-react";
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { api } from "@/api/client";
import { RelativeTime } from "@/components/relative-time";
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
    () => ({
      version: 0,
      state: "loading" as const,
      lines: [] as LogLine[],
      job: null,
      lastOutputAt: null,
    }),
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
    <div
      className={cn(
        "relative flex min-h-0 flex-col overflow-hidden rounded-lg bg-code text-code-foreground ring-1 ring-code-line",
        className,
      )}
    >
      <div className="flex flex-wrap items-center gap-1.5 border-b border-code-line bg-code-raised px-2 py-1.5">
        <div className="relative">
          <Search className="pointer-events-none absolute top-1.5 left-2 size-3.5 text-code-muted" />
          <Input
            aria-label="Search the log"
            placeholder="Search"
            className="h-7 w-40 border-code-line bg-code pl-7 text-sm text-code-foreground placeholder:text-code-muted sm:w-52"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setCursor(-1);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") step(event.shiftKey ? -1 : 1);
            }}
          />
        </div>
        <span aria-live="polite" className="tabular min-w-14 text-sm text-code-muted">
          {query.trim() ? `${matches.length} match${matches.length === 1 ? "" : "es"}` : null}
        </span>
        <LogButton label="Previous" disabled={matches.length === 0} onClick={() => step(-1)}>
          <ChevronUp />
        </LogButton>
        <LogButton label="Next" disabled={matches.length === 0} onClick={() => step(1)}>
          <ChevronDown />
        </LogButton>
        <button
          type="button"
          disabled={first < 0}
          onClick={() => goTo(first)}
          className="flex h-7 items-center gap-1.5 rounded-md px-2 text-sm text-code-muted hover:bg-code-line hover:text-code-foreground disabled:opacity-40"
        >
          <TriangleAlert className="size-3.5" />
          Jump to first error
        </button>
        <label className="ml-auto flex items-center gap-1.5 text-sm text-code-muted">
          <input
            type="checkbox"
            className="size-3.5 accent-[var(--code-foreground)]"
            checked={follow}
            onChange={(event) => setFollow(event.target.checked)}
          />
          Follow
        </label>
        <span role="status" className="flex items-center gap-1.5 text-sm text-code-muted">
          <span
            aria-hidden
            className={cn(
              "size-1.5 rounded-full",
              log.state === "streaming" && "animate-pulse-dot bg-running",
              log.state === "ended" && "bg-passed",
              log.state === "failed" && "animate-pulse-dot bg-errored",
              log.state === "loading" && "bg-code-muted",
            )}
          />
          {STATE_TEXT[log.state]}
          {log.state === "streaming" && log.lastOutputAt !== null ? (
            <span className="text-code-muted">
              · last output <RelativeTime at={new Date(log.lastOutputAt).toISOString()} />
            </span>
          ) : null}
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
        <p className="flex items-center gap-2 bg-code px-4 pb-4 text-base text-code-muted">
          {log.state === "ended" ? "The job printed nothing." : "Waiting for output…"}
        </p>
      ) : null}
      {!follow && log.state === "streaming" ? (
        <button
          type="button"
          className="absolute right-4 bottom-4 flex items-center gap-1.5 rounded-full bg-code-foreground px-3 py-1.5 text-sm font-medium text-code shadow-float"
          onClick={() => setFollow(true)}
        >
          <ArrowDownToLine className="size-3.5" />
          Jump to latest
        </button>
      ) : null}
    </div>
  );
}

function LogButton({
  label,
  disabled,
  onClick,
  children,
}: {
  label: string;
  disabled: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onClick}
      className="flex size-7 items-center justify-center rounded-md text-code-muted hover:bg-code-line hover:text-code-foreground disabled:opacity-40 [&_svg]:size-4"
    >
      {children}
    </button>
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
        "flex h-5 gap-4 pr-4 font-mono text-sm leading-5 whitespace-pre",
        line.stream === "stderr" && "text-code-error",
        highlighted && "bg-code-line",
      )}
    >
      <span
        aria-hidden
        className="numeric w-12 shrink-0 pr-1 text-right text-code-muted/70 select-none"
      >
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
      <mark key={at} className="rounded-sm bg-code-mark text-black">
        {text.slice(at, at + needle.length)}
      </mark>,
    );
    from = at + needle.length;
  }
  parts.push(text.slice(from));
  return <>{parts}</>;
}
