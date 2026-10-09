import type { EventNotice } from "./event-stream";

/** The most recent events the console received, newest first, kept for the Now page. */
export class ActivityLog {
  private notices: readonly EventNotice[] = [];
  private readonly listeners = new Set<() => void>();

  constructor(private readonly capacity = 50) {}

  /** Records `notice`; the oldest notice beyond the capacity is dropped. */
  add(notice: EventNotice): void {
    this.notices = [notice, ...this.notices].slice(0, this.capacity);
    for (const listener of this.listeners) listener();
  }

  /** The notices now; the same array until the next `add`. */
  getSnapshot = (): readonly EventNotice[] => this.notices;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
}
