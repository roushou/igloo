import { Terminal } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { Button } from "@/components/ui/button";

/**
 * A command the user can run from the console. While `unmet` names a rule that is not met, the
 * button is disabled and the rule is written beside it. The terminal button opens a panel with
 * the equivalent `igloo` command.
 */
export function ActionButton({
  label,
  cli,
  onRun,
  unmet,
  pending,
  variant = "outline",
  type = "button",
}: {
  label: string;
  cli: string;
  onRun: () => void;
  unmet?: string | null;
  pending?: boolean;
  variant?: "default" | "outline" | "ghost";
  /** A `submit` action runs by submitting its form, so `onRun` is not called. */
  type?: "button" | "submit";
}) {
  const reasonId = useId();
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (event: Event) => {
      if (
        event instanceof KeyboardEvent
          ? event.key === "Escape"
          : !root.current?.contains(event.target as Node)
      ) {
        setOpen(false);
      }
    };
    document.addEventListener("keydown", close);
    document.addEventListener("pointerdown", close);
    return () => {
      document.removeEventListener("keydown", close);
      document.removeEventListener("pointerdown", close);
    };
  }, [open]);

  return (
    <div ref={root} className="relative flex flex-col items-start gap-1">
      <div className="flex items-center">
        <Button
          variant={variant}
          size="sm"
          className="rounded-r-none"
          type={type}
          disabled={Boolean(unmet) || pending}
          aria-describedby={unmet ? reasonId : undefined}
          onClick={type === "submit" ? undefined : onRun}
        >
          {pending ? `${label}…` : label}
        </Button>
        <Button
          variant="outline"
          size="icon"
          className="size-8 rounded-l-none border-l-0"
          type="button"
          aria-label={`CLI command for ${label}`}
          aria-expanded={open}
          onClick={() => setOpen(!open)}
        >
          <Terminal className="size-3.5" />
        </Button>
      </div>
      {open ? (
        <div className="absolute top-full right-0 z-50 mt-1 w-96 max-w-[80vw] rounded-lg border bg-card p-3 shadow-lg">
          <p className="mb-1 text-xs text-muted-foreground">The same from a terminal</p>
          <CliCommand command={cli} />
        </div>
      ) : null}
      {unmet ? (
        <span id={reasonId} className="max-w-xs text-xs text-muted-foreground">
          {unmet}
        </span>
      ) : null}
    </div>
  );
}

/** A shell command on the dark surface, copied by its button. */
export function CliCommand({ command }: { command: string }) {
  return (
    <div className="flex items-start gap-2 rounded-md bg-code px-3 py-2 text-code-foreground">
      <code className="min-w-0 flex-1 text-xs break-all whitespace-pre-wrap">{command}</code>
      <button
        type="button"
        className="shrink-0 text-xs text-code-muted hover:text-code-foreground"
        onClick={() => void navigator.clipboard?.writeText(command)}
      >
        Copy
      </button>
    </div>
  );
}
