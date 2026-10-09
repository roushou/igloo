import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { CliCommand } from "@/components/action-button";

/**
 * What a list shows when it has nothing: what will appear here, and how to make it happen from a
 * terminal or an agent.
 */
export function EmptyState({
  icon: Icon,
  title,
  children,
  command,
  hint,
}: {
  icon: LucideIcon;
  title: string;
  children: ReactNode;
  /** The `igloo` command that fills the list. */
  command?: string;
  /** Other ways in, such as the MCP tool. */
  hint?: ReactNode;
}) {
  return (
    <div className="mx-auto flex max-w-md flex-col items-center gap-3 px-4 py-12 text-center">
      <span className="flex size-10 items-center justify-center rounded-full border bg-muted text-muted-foreground">
        <Icon className="size-5" />
      </span>
      <div className="flex flex-col gap-1">
        <p className="text-lg font-semibold">{title}</p>
        <p className="text-base text-muted-foreground">{children}</p>
      </div>
      {command ? (
        <div className="w-full text-left">
          <CliCommand command={command} />
        </div>
      ) : null}
      {hint ? <p className="text-sm text-muted-foreground">{hint}</p> : null}
    </div>
  );
}
