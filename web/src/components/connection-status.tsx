import type { ConnectionState } from "@/lib/event-stream";
import { useConnectionState } from "@/lib/events";
import { cn } from "@/lib/utils";

const LABELS: Record<ConnectionState, string> = {
  live: "Live",
  reconnecting: "Reconnecting…",
  offline: "Offline",
};

const DOTS: Record<ConnectionState, string> = {
  live: "bg-passed",
  reconnecting: "bg-errored animate-pulse-dot",
  offline: "bg-failed",
};

/** Whether live updates are arriving; `compact` shows the dot alone. */
export function ConnectionStatus({ compact }: { compact?: boolean }) {
  const state = useConnectionState();
  return (
    <p
      role="status"
      title={LABELS[state]}
      className="flex items-center gap-2 px-2 text-sm text-muted-foreground"
    >
      <span aria-hidden className={cn("size-2 shrink-0 rounded-full", DOTS[state])} />
      <span className={cn(compact && "sr-only")}>{LABELS[state]}</span>
    </p>
  );
}
