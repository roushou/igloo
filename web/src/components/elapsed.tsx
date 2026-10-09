import type { Check } from "@/api/client";
import { format } from "@/lib/format";
import { timing } from "@/lib/timing";
import { useNow } from "@/lib/use-now";

/** The time since `since`, ticking every second. */
export function Elapsed({ since }: { since: string }) {
  const now = useNow();
  return <span className="tabular">{format.duration(now - new Date(since).getTime())}</span>;
}

/**
 * How long something took or has taken. While it runs it ticks from `startedAt`, with the time
 * since its last output when that is known; once it ended it shows the final duration.
 */
export function Timing({
  startedAt,
  endedAt,
  lastOutputAt,
}: {
  startedAt: string;
  /** When it ended; absent while it runs, or while the server does not say. */
  endedAt?: string | null;
  /** When it last printed a line; absent until the server says. */
  lastOutputAt?: string | null;
}) {
  const now = useNow();
  const end = endedAt ? new Date(endedAt).getTime() : now;
  return (
    <span className="tabular inline-flex items-center gap-2">
      <span>{format.duration(end - new Date(startedAt).getTime())}</span>
      {!endedAt && lastOutputAt ? (
        <span className="text-muted-foreground/80">
          output {format.duration(now - new Date(lastOutputAt).getTime())} ago
        </span>
      ) : null}
    </span>
  );
}

/** How long a check ran or has run, once the server reports its times; nothing before. */
export function CheckTime({ check }: { check: Check }) {
  const times = timing.check(check);
  return times ? (
    <span className="text-sm text-muted-foreground">
      <Timing {...times} />
    </span>
  ) : null;
}
