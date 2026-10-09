import { cn } from "@/lib/utils";

/** A labelled bar showing `value` of `max`; amber from 90% on. */
export function Meter({
  label,
  value,
  max,
  text,
}: {
  label: string;
  value: number;
  max: number;
  /** What the bar reads, such as `6.0 GiB of 20 GiB`. */
  text: string;
}) {
  const ratio = max > 0 ? Math.min(1, value / max) : 0;
  return (
    <div className="flex flex-col gap-1.5 text-sm">
      <div className="flex justify-between gap-2">
        <span className="text-muted-foreground">{label}</span>
        <span className="tabular">{text}</span>
      </div>
      <meter
        className="sr-only"
        aria-label={label}
        min={0}
        max={max}
        value={value}
        aria-valuetext={text}
      />
      <div aria-hidden className="h-1 overflow-hidden rounded-full bg-muted">
        <div
          className={cn(
            "h-full rounded-full transition-[width] duration-500",
            ratio >= 0.9 ? "bg-errored" : "bg-foreground/70",
          )}
          style={{ width: `${ratio * 100}%` }}
        />
      </div>
    </div>
  );
}
