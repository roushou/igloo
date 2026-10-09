import { Check, Copy, Terminal } from "lucide-react";
import { type ReactNode, useId, useState } from "react";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Tooltip } from "@/components/ui/tooltip";
import { clipboard } from "@/lib/clipboard";

/**
 * A command the user can run from the console. While `unmet` names a rule that is not met, the
 * button is disabled; the rule shows on hover and focus and is read as its description.
 */
export function ActionButton({
  label,
  text,
  onRun,
  unmet,
  pending,
  variant = "outline",
  size = "default",
  icon,
  type = "button",
}: {
  /** What the button is called for assistive technology, and shows unless `text` is given. */
  label: string;
  /** A shorter text to show; `label` stays the accessible name. */
  text?: string;
  onRun: () => void;
  unmet?: string | null;
  pending?: boolean;
  variant?: "default" | "expedition" | "outline" | "ghost" | "danger";
  size?: "sm" | "default" | "lg";
  icon?: ReactNode;
  /** A `submit` action runs by submitting its form, so `onRun` is not called. */
  type?: "button" | "submit";
}) {
  const reasonId = useId();
  return (
    <>
      <Tooltip content={unmet}>
        {/* A disabled button takes no hover, so the wrapper carries the tooltip. */}
        <span
          className="inline-flex"
          tabIndex={unmet ? 0 : undefined}
          aria-describedby={unmet ? reasonId : undefined}
        >
          <Button
            variant={variant}
            size={size}
            type={type}
            aria-label={text ? label : undefined}
            disabled={Boolean(unmet) || pending}
            aria-describedby={unmet ? reasonId : undefined}
            onClick={type === "submit" ? undefined : onRun}
          >
            {icon}
            {pending ? `${text ?? label}…` : (text ?? label)}
          </Button>
        </span>
      </Tooltip>
      {unmet ? (
        <span id={reasonId} className="sr-only">
          {unmet}
        </span>
      ) : null}
    </>
  );
}

/** A shell command on the dark surface, copied by its button. */
export function CliCommand({ command }: { command: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="flex items-center gap-2 rounded-md bg-code py-2 pr-1.5 pl-3 text-code-foreground">
      <span aria-hidden className="font-mono text-sm text-code-muted select-none">
        $
      </span>
      <code className="min-w-0 flex-1 overflow-x-auto font-mono text-sm whitespace-pre">
        {command}
      </code>
      <button
        type="button"
        aria-label="Copy the command"
        className="shrink-0 rounded p-1 text-code-muted hover:bg-code-line hover:text-code-foreground"
        onClick={async () => {
          if (!(await clipboard.copy(command))) return;
          setCopied(true);
          setTimeout(() => setCopied(false), 1_500);
        }}
      >
        {copied ? (
          <Check className="size-3.5 text-code-added-foreground" />
        ) : (
          <Copy className="size-3.5" />
        )}
      </button>
    </div>
  );
}

/** The commands of a page's actions, each with the `igloo` command that does the same. */
export function CliMenu({ commands }: { commands: { label: string; command: string }[] }) {
  return (
    <Popover>
      <Tooltip content="The same from a terminal">
        <PopoverTrigger asChild>
          <Button variant="ghost" size="icon" aria-label="Terminal commands">
            <Terminal />
          </Button>
        </PopoverTrigger>
      </Tooltip>
      <PopoverContent className="flex flex-col gap-3">
        <p className="text-sm text-muted-foreground">The same from a terminal</p>
        {commands.map((entry) => (
          <div key={entry.label} className="flex flex-col gap-1">
            <p className="text-sm font-medium">{entry.label}</p>
            <CliCommand command={entry.command} />
          </div>
        ))}
      </PopoverContent>
    </Popover>
  );
}
