import { Check } from "lucide-react";
import { useState } from "react";
import { useToast } from "@/components/ui/toast";
import { clipboard } from "@/lib/clipboard";
import { format } from "@/lib/format";
import { cn } from "@/lib/utils";

/** An id shortened for reading; clicking copies the whole id. */
export function ShortId({ id, className }: { id: string; className?: string }) {
  const [copied, setCopied] = useState(false);
  const toast = useToast();
  return (
    <button
      type="button"
      title={id}
      aria-label={`Copy ${id}`}
      className={cn(
        "relative z-10 inline-flex items-center gap-1 rounded px-1 font-mono text-sm text-muted-foreground hover:bg-accent hover:text-foreground",
        className,
      )}
      onClick={async (event) => {
        event.preventDefault();
        event.stopPropagation();
        if (!(await clipboard.copy(id))) {
          toast.show({ title: "Could not copy", description: id, tone: "error" });
          return;
        }
        setCopied(true);
        toast.show({ title: "Copied", description: id });
        setTimeout(() => setCopied(false), 1_500);
      }}
    >
      {copied ? (
        <>
          <Check className="size-3 text-passed" />
          copied
        </>
      ) : (
        format.shortId(id)
      )}
    </button>
  );
}
