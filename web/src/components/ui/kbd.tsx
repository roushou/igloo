import { cn } from "@/lib/utils";

/** A key cap. */
export function Kbd({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <kbd
      className={cn(
        "inline-flex h-5 min-w-5 items-center justify-center rounded border bg-muted px-1 text-xs font-medium text-muted-foreground",
        className,
      )}
    >
      {children}
    </kbd>
  );
}
