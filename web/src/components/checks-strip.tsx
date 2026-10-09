import type { Check } from "@/api/client";
import { STATE_LABELS, status } from "@/lib/status";
import { cn } from "@/lib/utils";

const COLORS = {
  "needs-you": "bg-expedition",
  running: "bg-running animate-pulse-dot",
  passed: "bg-passed",
  failed: "bg-failed",
  errored: "bg-errored",
  closed: "bg-closed",
} as const;

/** One small segment per check, in the colour of its state: how far a run has got, at a glance. */
export function ChecksStrip({
  checks,
  className,
}: {
  checks: readonly Check[];
  className?: string;
}) {
  return (
    <span
      role="img"
      aria-label={checks
        .map((c) => `${c.name} ${STATE_LABELS[status.check(c)].toLowerCase()}`)
        .join(", ")}
      className={cn("inline-flex items-center gap-0.5", className)}
    >
      {checks.map((check) => {
        const state = status.check(check);
        const pending = check.status === "pending";
        return (
          <span
            key={check.name}
            title={`${check.name}: ${STATE_LABELS[state]}`}
            className={cn("h-1.5 w-4 rounded-full", pending ? "bg-border" : COLORS[state])}
          />
        );
      })}
    </span>
  );
}
