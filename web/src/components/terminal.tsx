import { useEffect, useRef, useState } from "react";
import { api } from "@/api/client";
import { StatusPill } from "@/components/status-pill";
import { TerminalConnection, type TerminalEnd, type TerminalSnapshot } from "@/lib/terminal";
import { cn } from "@/lib/utils";

const NO_SNAPSHOT: TerminalSnapshot = { version: 0, state: "connecting", end: null };

/** What the strip under a finished terminal says about how its process ended. */
function describeEnd(end: TerminalEnd | null): { text: string; failed: boolean } {
  if (end && "code" in end) {
    return end.code === 0
      ? { text: "Exited", failed: false }
      : { text: `Exited with code ${end.code}`, failed: true };
  }
  switch (end?.failure) {
    case "sandbox_unavailable":
      return { text: "The sandbox is not running", failed: true };
    case "execution_error":
      return { text: "The shell could not start", failed: true };
    case "closed":
      return { text: "Closed", failed: false };
    default:
      return { text: "Connection to the worker lost", failed: true };
  }
}

/** Reads the code surface's colours and the mono font from the tokens of `host`. */
function themeOf(host: HTMLElement) {
  const style = getComputedStyle(host);
  const token = (name: string) => style.getPropertyValue(name).trim();
  return {
    fontFamily: style.fontFamily,
    fontSize: Number.parseFloat(style.fontSize),
    theme: {
      background: token("--code"),
      foreground: token("--code-foreground"),
      cursor: token("--code-foreground"),
      cursorAccent: token("--code"),
      selectionBackground: token("--code-line"),
      brightBlack: token("--code-muted"),
      red: token("--code-error"),
      brightRed: token("--code-error"),
      green: token("--code-added-foreground"),
      brightGreen: token("--code-added-foreground"),
      yellow: token("--code-mark"),
      brightYellow: token("--code-mark"),
    },
  };
}

/**
 * An interactive terminal in a running sandbox, on the dark code surface. It opens a shell (or
 * `command`) when it mounts, fits itself to its box, and ends the process when it unmounts.
 * `command` is read once: remount with a `key` to run something else. `onExit` is called once
 * when the process ends.
 */
export function Terminal({
  sandboxId,
  command,
  onExit,
  className,
}: {
  sandboxId: string;
  command?: string[];
  onExit?: (end: TerminalEnd | null) => void;
  className?: string;
}) {
  const host = useRef<HTMLElement>(null);
  const spawn = useRef({ command, onExit });
  spawn.current = { command, onExit };
  const [snapshot, setSnapshot] = useState(NO_SNAPSHOT);

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    let disposed = false;
    let dispose = () => {};
    // xterm touches the browser when imported, and the build imports every module on the server.
    void Promise.all([import("@xterm/xterm"), import("@xterm/addon-fit")]).then(
      ([{ Terminal: Screen }, { FitAddon }]) => {
        if (disposed) return;
        const screen = new Screen({
          ...themeOf(element),
          cursorBlink: true,
          scrollback: 5_000,
          allowProposedApi: false,
        });
        const fit = new FitAddon();
        screen.loadAddon(fit);
        screen.open(element);
        fit.fit();
        const live = new TerminalConnection(
          (size) => api.terminalSocket(sandboxId, { command: spawn.current.command, ...size }),
          (data) => screen.write(data),
          { cols: screen.cols, rows: screen.rows },
        );
        const unsubscribe = live.subscribe(() => {
          const snapshot = live.getSnapshot();
          const { state, end } = snapshot;
          if (state === "open") screen.focus();
          if (state === "exited") spawn.current.onExit?.(end);
          if (state !== "open" && state !== "connecting") screen.options.disableStdin = true;
          setSnapshot(snapshot);
        });
        const typed = screen.onData((data) => live.type(data));
        const binary = screen.onBinary((data) =>
          live.type(Uint8Array.from(data, (c) => c.charCodeAt(0))),
        );
        const resized = screen.onResize(({ cols, rows }) => live.resize({ cols, rows }));
        const observer = new ResizeObserver(() => fit.fit());
        observer.observe(element);
        live.start();
        dispose = () => {
          observer.disconnect();
          typed.dispose();
          binary.dispose();
          resized.dispose();
          unsubscribe();
          live.close();
          screen.dispose();
        };
      },
    );
    return () => {
      disposed = true;
      dispose();
    };
  }, [sandboxId]);

  const finished = snapshot.state !== "connecting" && snapshot.state !== "open";
  const ended = snapshot.state === "exited" ? describeEnd(snapshot.end) : null;
  return (
    <div
      className={cn(
        "flex min-h-0 flex-col overflow-hidden rounded-md bg-code text-code-foreground ring-1 ring-code-line",
        className,
      )}
    >
      <section
        ref={host}
        aria-label="Terminal"
        className="min-h-0 flex-1 p-2 font-mono text-base"
      />
      {snapshot.state === "connecting" || finished ? (
        <p
          role="status"
          className="flex items-center gap-2 border-t border-code-line bg-code-raised px-3 py-1.5 text-sm text-code-muted"
        >
          {snapshot.state === "connecting" ? "Connecting…" : null}
          {ended ? (
            <StatusPill state={ended.failed ? "failed" : "closed"} label={ended.text} />
          ) : null}
          {snapshot.state === "closed" ? "Disconnected" : null}
          {snapshot.state === "failed" ? "Could not open the terminal" : null}
        </p>
      ) : null}
    </div>
  );
}
