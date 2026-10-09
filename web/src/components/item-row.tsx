import { Link, type LinkProps } from "@tanstack/react-router";
import { motion } from "motion/react";
import type { ReactNode } from "react";
import { StatusPill } from "@/components/status-pill";
import { useMotion } from "@/lib/motion";
import type { State } from "@/lib/status";
import { cn } from "@/lib/utils";

/**
 * One row of a list: a state, a title that links to the item, what it waits on under the title,
 * and meta and time on the right. The whole row opens the item; `j` and `k` move between rows.
 * Rows lay out by the width of their list, not of the screen: in a narrow list the title takes a
 * line of its own under the pill.
 */
export function ItemRow({
  state,
  label,
  title,
  link,
  details,
  meta,
  time,
  selected,
}: {
  state: State;
  /** Replaces the state's name on the pill. */
  label?: string;
  title: ReactNode;
  link: Pick<LinkProps, "to" | "params" | "search">;
  /** A line under the title, such as what the item waits on. */
  details?: ReactNode;
  /** Ids and other facts, in muted mono. */
  meta?: ReactNode;
  /** When it happened or how long it has run. */
  time?: ReactNode;
  /** The item whose detail is open beside the list. */
  selected?: boolean;
}) {
  const { transition } = useMotion();
  return (
    <motion.li
      layout="position"
      initial={{ opacity: 0, y: -4 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0 }}
      transition={transition(0.18)}
      aria-current={selected ? "true" : undefined}
      className={cn(
        "group relative grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-3 gap-y-1 px-4 py-2.5 transition-colors duration-150 hover:bg-accent/60 has-[a:focus-visible]:bg-accent has-[a:focus-visible]:outline-2 has-[a:focus-visible]:-outline-offset-2 has-[a:focus-visible]:outline-ring/70 @xl:grid-cols-[6rem_minmax(0,1fr)_auto]",
        selected && "bg-accent",
        state === "needs-you" &&
          "before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-expedition",
      )}
    >
      <StatusPill state={state} label={label} className="justify-self-start" />
      <div className="col-span-2 row-start-2 min-w-0 @xl:col-span-1 @xl:col-start-2 @xl:row-start-1">
        <Link
          {...link}
          data-nav-row
          className="block truncate text-base font-medium outline-none after:absolute after:inset-0"
        >
          {title}
        </Link>
        {details ? (
          <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-sm text-muted-foreground">
            {details}
          </div>
        ) : null}
      </div>
      {meta || time ? (
        <div className="col-start-2 row-start-1 flex items-center justify-self-end text-sm text-muted-foreground @xl:col-start-3">
          {meta ? <span className="mr-3 flex items-center gap-2 font-mono">{meta}</span> : null}
          {time ? <span className="tabular min-w-14 text-right">{time}</span> : null}
        </div>
      ) : null}
    </motion.li>
  );
}
