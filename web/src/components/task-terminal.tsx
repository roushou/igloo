import { useState } from "react";
import type { Task, TerminalMode } from "@/api/client";
import { Terminal } from "@/components/terminal";
import { Button } from "@/components/ui/button";

/** What each stage of a takeover tells the person. */
const TAKEOVER_TEXT = {
  waiting:
    "You took the task over. Its current turn is finishing; no new turn will start until you hand it back.",
  paused: "You took the task over. It is paused until you hand it back.",
  handing_back: "Handing the task back: reading what you changed in the sandbox.",
} as const;

/**
 * A task's sandbox: its terminal, read-only until the person takes the task over, then writable
 * until they hand it back. The terminal opens when asked for, or at once for a task that is taken
 * over; the server drops what a read-only terminal is sent, so the notice below it is the whole
 * of the UI's part. A task that ended has no sandbox to show.
 */
export function TaskTerminal({ task }: { task: Task }) {
  const [watching, setWatching] = useState(false);
  const { takeover, sandbox } = task;
  const ended = task.phase === "done" || task.phase === "failed" || task.phase === "cancelled";
  if (!sandbox || ended) return null;

  const typing = takeover !== undefined && takeover !== null && takeover.phase !== "handing_back";
  const mode: TerminalMode = typing ? "read_write" : "read_only";
  const shown = watching || Boolean(takeover);
  return (
    <section aria-label="Sandbox" className="flex flex-col gap-2">
      <div className="flex h-8 items-center justify-between">
        <h2 className="font-sans text-base font-semibold">Sandbox</h2>
        {takeover ? null : (
          <Button variant="ghost" size="sm" onClick={() => setWatching(!watching)}>
            {watching ? "Hide terminal" : "Watch the sandbox"}
          </Button>
        )}
      </div>
      {takeover ? (
        <p className="text-base text-muted-foreground">{TAKEOVER_TEXT[takeover.phase]}</p>
      ) : null}
      {shown ? (
        <>
          <Terminal key={mode} sandboxId={sandbox} mode={mode} className="h-80" />
          <p className="text-sm text-muted-foreground">
            {typing
              ? "Your keystrokes go to the sandbox."
              : "Read-only: what you type is not sent. Take the task over to type."}
          </p>
        </>
      ) : null}
    </section>
  );
}
