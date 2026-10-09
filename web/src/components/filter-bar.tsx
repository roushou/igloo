import { Search } from "lucide-react";
import { Input } from "@/components/ui/input";
import { STATE_LABELS, type State } from "@/lib/status";
import { cn } from "@/lib/utils";

/**
 * The filters of a list: one chip per state with its count, and a text box. Both live in the URL,
 * so a filtered list can be shared and survives a reload.
 */
export function FilterBar({
  counts,
  state,
  onState,
  query,
  onQuery,
  what,
}: {
  /** How many items each state has; states without any are not offered. */
  counts: Partial<Record<State, number>>;
  state: State | undefined;
  onState: (state: State | undefined) => void;
  query: string;
  onQuery: (query: string) => void;
  what: string;
}) {
  const total = Object.values(counts).reduce((sum, n) => sum + n, 0);
  const shown = (Object.keys(counts) as State[]).filter((s) => (counts[s] ?? 0) > 0 || s === state);
  return (
    <div className="flex flex-col gap-2">
      <fieldset className="flex min-w-0 flex-wrap gap-1">
        <legend className="sr-only">Filter {what} by state</legend>
        <Chip active={state === undefined} onClick={() => onState(undefined)}>
          All <span className="tabular text-muted-foreground">{total}</span>
        </Chip>
        {shown.map((s) => (
          <Chip key={s} active={state === s} onClick={() => onState(state === s ? undefined : s)}>
            {STATE_LABELS[s]}{" "}
            <span className="tabular text-muted-foreground">{counts[s] ?? 0}</span>
          </Chip>
        ))}
      </fieldset>
      <div className="relative">
        <Search className="pointer-events-none absolute top-2 left-2.5 size-4 text-muted-foreground" />
        <Input
          aria-label={`Filter ${what}`}
          placeholder={`Filter ${what}`}
          className="pl-8"
          value={query}
          onChange={(event) => onQuery(event.target.value)}
        />
      </div>
    </div>
  );
}

function Chip({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "flex h-7 shrink-0 items-center gap-1.5 rounded-full border px-2.5 text-sm font-medium text-muted-foreground transition-colors hover:text-foreground",
        active && "border-foreground/20 bg-accent text-foreground",
      )}
    >
      {children}
    </button>
  );
}
