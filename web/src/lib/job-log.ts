import type { Job } from "@/api/client";
import { SseParser } from "./sse";

/** One line of a job's output. */
export type LogLine = { text: string; stream: "stdout" | "stderr" };

/** Where reading a job's output is. */
export type LogState = "loading" | "streaming" | "ended" | "failed";

export type LogSnapshot = {
  /** Bumped by every change, so equal snapshots are the same object. */
  version: number;
  state: LogState;
  lines: readonly LogLine[];
  /** The finished job, from the stream's last event. */
  job: Job | null;
};

/** Opens a job's output stream after event `lastId` (none the first time). */
export type OpenLog = (
  lastId: string | null,
  signal: AbortSignal,
) => Promise<ReadableStream<Uint8Array>>;

// biome-ignore lint/suspicious/noControlCharactersInRegex: matching terminal escape sequences is the point.
const ESCAPES = /\u001b\[[0-9;?]*[ -/]*[@-~]/g;

/**
 * A job's output read from its server-sent stream into lines, following the job while it runs.
 * A dropped connection is reopened after the last event seen, so no output repeats or is lost.
 * Terminal escape sequences are removed; a trailing partial line is shown until it completes.
 */
export class JobLog {
  private readonly lines: LogLine[] = [];
  private readonly partial: Record<LogLine["stream"], string> = { stdout: "", stderr: "" };
  private state: LogState = "loading";
  private job: Job | null = null;
  private lastId: string | null = null;
  private abort: AbortController | null = null;
  private snapshot: LogSnapshot = { version: 0, state: "loading", lines: [], job: null };
  private readonly listeners = new Set<() => void>();

  constructor(
    private readonly open: OpenLog,
    private readonly retryMs = 1_000,
  ) {}

  getSnapshot = (): LogSnapshot => this.snapshot;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  /** Starts reading; does nothing when already started. */
  start(): void {
    if (this.abort) return;
    const abort = new AbortController();
    this.abort = abort;
    void this.run(abort.signal);
  }

  stop(): void {
    this.abort?.abort();
    this.abort = null;
  }

  private isEnded(): boolean {
    return this.state === "ended";
  }

  private publish(): void {
    const partial = [
      ...(this.partial.stdout ? [{ text: this.partial.stdout, stream: "stdout" as const }] : []),
      ...(this.partial.stderr ? [{ text: this.partial.stderr, stream: "stderr" as const }] : []),
    ];
    this.snapshot = {
      version: this.snapshot.version + 1,
      state: this.state,
      lines: partial.length ? [...this.lines, ...partial] : this.lines.slice(),
      job: this.job,
    };
    for (const listener of this.listeners) listener();
  }

  private append(stream: LogLine["stream"], chunk: string): void {
    const parts = (this.partial[stream] + chunk.replace(ESCAPES, "")).split(/\r?\n/);
    this.partial[stream] = parts.pop() ?? "";
    for (const text of parts) this.lines.push({ text, stream });
  }

  private flushPartials(): void {
    for (const stream of ["stdout", "stderr"] as const) {
      if (this.partial[stream]) this.lines.push({ text: this.partial[stream], stream });
      this.partial[stream] = "";
    }
  }

  private async run(signal: AbortSignal): Promise<void> {
    while (!signal.aborted && !this.isEnded()) {
      try {
        const body = await this.open(this.lastId, signal);
        this.state = "streaming";
        this.publish();
        await this.read(body);
      } catch {
        // A failed or dropped connection is retried below.
      }
      if (signal.aborted || this.isEnded()) return;
      this.state = "failed";
      this.publish();
      await new Promise<void>((resolve) => {
        const timer = setTimeout(resolve, this.retryMs);
        signal.addEventListener("abort", () => {
          clearTimeout(timer);
          resolve();
        });
      });
    }
  }

  private async read(body: ReadableStream<Uint8Array>): Promise<void> {
    const reader = body.getReader();
    const decoder = new TextDecoder();
    const parser = new SseParser();
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) return;
        let changed = false;
        for (const frame of parser.feed(decoder.decode(value, { stream: true }))) {
          if (frame.id !== null) this.lastId = frame.id;
          if (frame.event === "stdout" || frame.event === "stderr") {
            this.append(frame.event, frame.data);
            changed = true;
          } else if (frame.event === "end") {
            this.flushPartials();
            this.job = JSON.parse(frame.data) as Job;
            this.state = "ended";
            changed = true;
          } else if (frame.event === "error") {
            this.state = "failed";
            changed = true;
          }
        }
        if (changed) this.publish();
        if (this.isEnded()) return;
      }
    } finally {
      reader.releaseLock();
    }
  }
}

/** The index of the first line that mentions `error`, or -1. */
export function firstErrorLine(lines: readonly LogLine[]): number {
  return lines.findIndex((line) => /error/i.test(line.text));
}
