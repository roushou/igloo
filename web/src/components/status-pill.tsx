import { motion } from "motion/react";
import { useMotion } from "@/lib/motion";
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

/**
 * The one pill every page uses for a state; `label` replaces the state's name when given. A
 * running pill's dot breathes; a pill that changes state pops into its new colour.
 */
export function StatusPill({
  state,
  label,
  className,
}: {
  state: State;
  label?: string;
  className?: string;
}) {
  const { transition } = useMotion();
  return (
    <motion.span
      key={state}
      data-state={state}
      initial={{ scale: 0.92, opacity: 0.4 }}
      animate={{ scale: 1, opacity: 1 }}
      transition={transition(0.2)}
      className={cn(
        "inline-flex h-5 shrink-0 items-center gap-1.5 rounded-full px-2 text-sm font-medium whitespace-nowrap",
        STYLES[state],
        className,
      )}
    >
      <span
        aria-hidden
        className={cn(
          "size-1.5 rounded-full bg-current",
          state === "running" && "animate-pulse-dot",
        )}
      />
      {label ?? STATE_LABELS[state]}
    </motion.span>
  );
}

/** A bare dot in a state's colour, for places too small for a pill. */
export function StateDot({ state, className }: { state: State; className?: string }) {
  const color: Record<State, string> = {
    "needs-you": "bg-expedition",
    running: "bg-running",
    passed: "bg-passed",
    failed: "bg-failed",
    errored: "bg-errored",
    closed: "bg-closed",
  };
  return (
    <span aria-hidden className={cn("size-2 shrink-0 rounded-full", color[state], className)} />
  );
}
