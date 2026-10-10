import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import type { Task, TerminalMode } from "@/api/client";
import { Terminal } from "@/components/terminal";
import { Button } from "@/components/ui/button";
import { format } from "@/lib/format";
import { queries } from "@/lib/queries";

/** What each stage of the viewer's own takeover tells them. */
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
 * of the UI's part. A task someone else took over stays read-only for the viewer, who is told
 * so and may hand it back. A task that ended has no sandbox to show.
 */
export function TaskTerminal({ task }: { task: Task }) {
  const [watching, setWatching] = useState(false);
  const me = useQuery(queries.me());
  const { takeover, sandbox } = task;
  const ended = task.phase === "done" || task.phase === "failed" || task.phase === "cancelled";
  if (!sandbox || ended) return null;

  // Without an answer from `/v1/me` the takeover is taken as the viewer's: the server decides.
  const others = Boolean(takeover && me.data && takeover.by !== me.data.id);
  const typing =
    takeover !== undefined && takeover !== null && takeover.phase !== "handing_back" && !others;
  const mode: TerminalMode = typing ? "read_write" : "read_only";
  // Whose takeover it is decides the mode, so the terminal waits for the answer instead of
  // opening in one mode and reopening in the other.
  const shown = (watching || Boolean(takeover)) && !(takeover && me.isPending);
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
        <p className="text-base text-muted-foreground">
          {others
            ? `${format.shortId(takeover.by)} took the task over, so the terminal is read-only for you. Hand the task back to release it.`
            : TAKEOVER_TEXT[takeover.phase]}
        </p>
      ) : null}
      {shown ? (
        <>
          <Terminal key={mode} sandboxId={sandbox} mode={mode} className="h-80" />
          <p className="text-sm text-muted-foreground">
            {typing
              ? "Your keystrokes go to the sandbox."
              : others
                ? "Read-only: only the person who took the task over can type."
                : "Read-only: what you type is not sent. Take the task over to type."}
          </p>
        </>
      ) : null}
    </section>
  );
}
