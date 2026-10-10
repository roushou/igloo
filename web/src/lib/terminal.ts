import type { TerminalEndReason, TerminalServerFrame } from "@/api/client";

/** Where a terminal connection is. */
export type TerminalState = "connecting" | "open" | "exited" | "closed" | "failed";

/** How the terminal's process ended: with an exit code, or without one for a reason. */
export type TerminalEnd = { code: number } | { failure: TerminalEndReason };

export type TerminalSnapshot = {
  /** Bumped by every change, so equal snapshots are the same object. */
  version: number;
  state: TerminalState;
  /** Set once `state` is `exited`. */
  end: TerminalEnd | null;
};

/** The screen size a terminal opens with and is resized to. */
export type TerminalSize = { cols: number; rows: number };

/** Opens the socket of a terminal that starts with `size`. */
export type OpenTerminalSocket = (size: TerminalSize) => WebSocket;

/**
 * One terminal session over a WebSocket: bytes typed go to the process, bytes printed come back
 * through `write`, and a resize is sent as soon as the socket is open. It ends with the process's
 * exit frame, or when the socket closes. It never reconnects: a new connection is a new shell.
 */
export class TerminalConnection {
  private socket: WebSocket | null = null;
  private state: TerminalState = "connecting";
  private end: TerminalEnd | null = null;
  private size: TerminalSize;
  private snapshot: TerminalSnapshot = { version: 0, state: "connecting", end: null };
  private readonly listeners = new Set<() => void>();
  private readonly encoder = new TextEncoder();

  constructor(
    private readonly open: OpenTerminalSocket,
    private readonly write: (data: Uint8Array) => void,
    size: TerminalSize,
  ) {
    this.size = size;
  }

  getSnapshot = (): TerminalSnapshot => this.snapshot;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  /** Opens the socket; does nothing when already started or closed. */
  start(): void {
    if (this.socket || this.state !== "connecting") return;
    let socket: WebSocket;
    try {
      socket = this.open(this.size);
    } catch {
      this.set("failed");
      return;
    }
    this.socket = socket;
    socket.addEventListener("open", () => this.set("open"));
    socket.addEventListener("message", (event) => this.receive(event.data));
    socket.addEventListener("close", () => {
      if (this.state === "connecting") this.set("failed");
      else if (this.state === "open") this.set("closed");
    });
  }

  /** Types `data` into the process; dropped unless the terminal is open. */
  type(data: string | Uint8Array): void {
    if (this.state !== "open") return;
    this.socket?.send(typeof data === "string" ? this.encoder.encode(data) : data);
  }

  /** Tells the process its screen is `size`. Sent when the socket opens if it is not yet. */
  resize(size: TerminalSize): void {
    if (size.cols === this.size.cols && size.rows === this.size.rows) return;
    this.size = size;
    if (this.state === "open") this.sendSize();
  }

  /** Ends the session, which kills the process. */
  close(): void {
    this.socket?.close(1000);
    this.socket = null;
    if (this.state === "connecting" || this.state === "open") this.set("closed");
  }

  private receive(data: unknown): void {
    if (data instanceof ArrayBuffer) {
      this.write(new Uint8Array(data));
      return;
    }
    if (typeof data !== "string") return;
    let frame: TerminalServerFrame;
    try {
      frame = JSON.parse(data) as TerminalServerFrame;
    } catch {
      return;
    }
    if (frame.type !== "exit") return;
    this.end =
      typeof frame.code === "number" ? { code: frame.code } : { failure: frame.failure ?? "lost" };
    this.set("exited");
  }

  private sendSize(): void {
    this.socket?.send(JSON.stringify({ type: "resize", ...this.size }));
  }

  private set(state: TerminalState): void {
    this.state = state;
    this.snapshot = { version: this.snapshot.version + 1, state, end: this.end };
    for (const listener of this.listeners) listener();
  }
}
