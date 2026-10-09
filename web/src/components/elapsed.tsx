import { format } from "@/lib/format";
import { useNow } from "@/lib/use-now";

/** The time since `since`, ticking every second. */
export function Elapsed({ since }: { since: string }) {
  const now = useNow();
  return <span className="tabular">{format.duration(now - new Date(since).getTime())}</span>;
}
