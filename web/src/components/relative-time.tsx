import { format } from "@/lib/format";
import { useNow } from "@/lib/use-now";

/** A time as `5m ago`, keeping itself current; hovering shows the absolute time. */
export function RelativeTime({ at, className }: { at: string; className?: string }) {
  const now = useNow(30_000);
  return (
    <time dateTime={at} title={new Date(at).toLocaleString()} className={className}>
      {format.relative(at, now)}
    </time>
  );
}
