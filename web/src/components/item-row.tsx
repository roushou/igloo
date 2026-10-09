import { Link, type LinkProps } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { StatusPill } from "@/components/status-pill";
import type { State } from "@/lib/status";
import { cn } from "@/lib/utils";

/** One row of a list: a state, a title that links to the item, and details under it. */
export function ItemRow({
  state,
  label,
  title,
  link,
  details,
  aside,
}: {
  state: State;
  /** Replaces the state's name on the pill. */
  label?: string;
  title: ReactNode;
  link: Pick<LinkProps, "to" | "params" | "search">;
  details?: ReactNode;
  aside?: ReactNode;
}) {
  return (
    <li
      className={cn(
        "flex items-start gap-3 px-4 py-3",
        state === "needs-you" && "border-l-2 border-l-expedition",
      )}
    >
      <StatusPill state={state} label={label} className="mt-0.5 w-24 justify-start" />
      <div className="min-w-0 flex-1">
        <Link {...link} className="line-clamp-2 text-sm font-medium hover:underline">
          {title}
        </Link>
        {details ? (
          <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-muted-foreground">
            {details}
          </div>
        ) : null}
      </div>
      {aside ? <div className="shrink-0 text-xs text-muted-foreground">{aside}</div> : null}
    </li>
  );
}
