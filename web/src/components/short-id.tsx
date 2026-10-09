import { useState } from "react";
import { format } from "@/lib/format";
import { cn } from "@/lib/utils";

/** An id shortened for reading; clicking copies the whole id. */
export function ShortId({ id, className }: { id: string; className?: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      title={id}
      aria-label={`Copy ${id}`}
      className={cn(
        "rounded font-mono text-xs text-muted-foreground hover:bg-accent hover:text-foreground",
        className,
      )}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        void navigator.clipboard?.writeText(id);
        setCopied(true);
        setTimeout(() => setCopied(false), 1_500);
      }}
    >
      {copied ? "copied" : format.shortId(id)}
    </button>
  );
}
