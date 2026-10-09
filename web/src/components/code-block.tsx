import { useMemo, useState } from "react";
import { cn } from "@/lib/utils";

/** Longer outputs show this many lines until the reader asks for the rest. */
const PREVIEW_LINES = 400;

/**
 * Text on the dark code surface with a line-number gutter. `tone="error"` tints it as an error;
 * `diff` colours lines that start with `+` or `-` as added and removed. A long text shows its
 * first lines and a button for the rest.
 */
export function CodeBlock({
  text,
  tone,
  diff,
  className,
  label,
}: {
  text: string;
  tone?: "error";
  diff?: boolean;
  className?: string;
  label?: string;
}) {
  const [all, setAll] = useState(false);
  const lines = useMemo(() => text.replace(/\n$/, "").split("\n"), [text]);
  const shown = all ? lines : lines.slice(0, PREVIEW_LINES);
  return (
    <div
      className={cn(
        "overflow-hidden rounded-md bg-code text-code-foreground ring-1 ring-code-line",
        className,
      )}
    >
      {label ? (
        <p className="truncate border-b border-code-line px-3 py-1.5 font-mono text-sm text-code-muted">
          {label}
        </p>
      ) : null}
      <pre className="max-h-96 overflow-auto py-1.5 font-mono text-sm leading-5">
        {shown.map((line, index) => {
          const added = diff && line.startsWith("+");
          const removed = diff && line.startsWith("-");
          return (
            <div
              // biome-ignore lint/suspicious/noArrayIndexKey: lines of a fixed text have no identity but their place.
              key={index}
              className={cn(
                "flex min-w-max px-0",
                added && "bg-code-added text-code-added-foreground",
                removed && "bg-code-removed text-code-removed-foreground",
                tone === "error" && !added && !removed && "text-code-error",
              )}
            >
              <span
                aria-hidden
                className="numeric w-10 shrink-0 pr-3 text-right text-code-muted/70 select-none"
              >
                {index + 1}
              </span>
              <span className="pr-4 whitespace-pre">{line || " "}</span>
            </div>
          );
        })}
        {shown.length < lines.length ? (
          <button
            type="button"
            className="mt-1 ml-10 text-sm text-code-muted underline hover:text-code-foreground"
            onClick={() => setAll(true)}
          >
            Show {lines.length - shown.length} more lines
          </button>
        ) : null}
      </pre>
    </div>
  );
}
