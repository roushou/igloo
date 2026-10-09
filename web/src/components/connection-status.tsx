import type { ConnectionState } from "@/lib/event-stream";
import { useConnectionState } from "@/lib/events";

const LABELS: Record<ConnectionState, string> = {
  live: "Live",
  reconnecting: "Reconnecting…",
  offline: "Offline",
};

const DOTS: Record<ConnectionState, string> = {
  live: "bg-green-500",
  reconnecting: "bg-amber-500",
  offline: "bg-red-500",
};

/** Whether live updates are arriving. */
export function ConnectionStatus() {
  const state = useConnectionState();
  return (
    <p role="status" className="mt-auto flex items-center gap-2 px-2 text-xs text-muted-foreground">
      <span aria-hidden className={`size-2 rounded-full ${DOTS[state]}`} />
      {LABELS[state]}
    </p>
  );
}
