import { STATE_LABELS, type State } from "@/lib/status";
import { cn } from "@/lib/utils";

const STYLES: Record<State, string> = {
  "needs-you": "bg-expedition-soft text-expedition",
  running: "bg-running-soft text-running",
  passed: "bg-passed-soft text-passed",
  failed: "bg-failed-soft text-failed",
  errored: "bg-errored-soft text-errored",
  closed: "bg-closed-soft text-closed",
};

/** The one pill every page uses for a state; `label` replaces the state's name when given. */
export function StatusPill({
  state,
  label,
  className,
}: {
  state: State;
  label?: string;
  className?: string;
}) {
  return (
    <span
      data-state={state}
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium",
        STYLES[state],
        className,
      )}
    >
      <span aria-hidden className="size-1.5 rounded-full bg-current" />
      {label ?? STATE_LABELS[state]}
    </span>
  );
}
