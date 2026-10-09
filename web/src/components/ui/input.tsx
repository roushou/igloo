import type * as React from "react";
import { cn } from "@/lib/utils";

function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      type={type}
      className={cn(
        "h-9 w-full min-w-0 rounded-md border bg-transparent px-3 text-sm shadow-xs outline-none placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring aria-invalid:border-destructive",
        className,
      )}
      {...props}
    />
  );
}

export { Input };
