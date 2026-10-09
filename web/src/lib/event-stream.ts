import type { components } from "@/api/schema.gen";
import { SseParser } from "./sse";

export type EventNotice = components["schemas"]["EventNotice"];

/** Whether the console is receiving events. */
export type ConnectionState = "live" | "reconnecting" | "offline";

/** Opens the event stream after `lastId` (none on the first connection); rejects when it cannot. */
export type OpenEvents = (
  lastId: string | null,
  signal: AbortSignal,
) => Promise<ReadableStream<Uint8Array>>;

export type EventStreamOptions = {
  open: OpenEvents;
  onNotice: (notice: EventNotice) => void;
  /** First retry delay in milliseconds; it doubles up to `maxDelayMs`. */
  initialDelayMs?: number;
  maxDelayMs?: number;
  /** Consecutive failed attempts after which the state is `offline`. */
  offlineAfter?: number;
};

/**
 * A resumable event stream. It reads until the server or network ends the stream, then reconnects
 * with exponential backoff from the last event id it saw, so no event is missed or repeated.
 * Reports `live` while connected, `reconnecting` for the first failures and `offline` after
 * several in a row or while the browser reports no network.
 */
export class EventStream {
  private readonly options: Required<EventStreamOptions>;
  private state: ConnectionState = "reconnecting";
  private readonly listeners = new Set<() => void>();
  private lastId: string | null = null;
  private abort: AbortController | null = null;

  constructor(options: EventStreamOptions) {
    this.options = {
      initialDelayMs: 1_000,
      maxDelayMs: 30_000,
      offlineAfter: 3,
      ...options,
    };
  }

  /** The connection state now. */
  getState = (): ConnectionState => this.state;

  /** Calls `listener` whenever the state changes; returns the unsubscribe function. */
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

  /** Stops reading and drops the connection. The id of the last event seen is kept. */
  stop(): void {
    this.abort?.abort();
    this.abort = null;
  }

  private setState(state: ConnectionState): void {
    if (state === this.state) return;
    this.state = state;
    for (const listener of this.listeners) listener();
  }

  private async run(signal: AbortSignal): Promise<void> {
    let failures = 0;
    let delay = this.options.initialDelayMs;
    while (!signal.aborted) {
      try {
        const body = await this.options.open(this.lastId, signal);
        failures = 0;
        delay = this.options.initialDelayMs;
        this.setState("live");
        await this.read(body);
      } catch {
        // An aborted or failed connection is retried below.
      }
      if (signal.aborted) return;
      failures += 1;
      const offline =
        failures >= this.options.offlineAfter ||
        (typeof navigator !== "undefined" && navigator.onLine === false);
      this.setState(offline ? "offline" : "reconnecting");
      await this.sleep(delay, signal);
      delay = Math.min(delay * 2, this.options.maxDelayMs);
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
        for (const frame of parser.feed(decoder.decode(value, { stream: true }))) {
          this.accept(frame.id, frame.data);
        }
      }
    } finally {
      reader.releaseLock();
    }
  }

  private accept(id: string | null, data: string): void {
    let notice: EventNotice;
    try {
      notice = JSON.parse(data) as EventNotice;
    } catch {
      return;
    }
    if (id !== null) this.lastId = id;
    this.options.onNotice(notice);
  }

  private sleep(ms: number, signal: AbortSignal): Promise<void> {
    return new Promise((resolve) => {
      const timer = setTimeout(done, ms);
      function done() {
        clearTimeout(timer);
        signal.removeEventListener("abort", done);
        resolve();
      }
      signal.addEventListener("abort", done);
    });
  }
}
