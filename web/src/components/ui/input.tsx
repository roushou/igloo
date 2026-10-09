import type * as React from "react";
import { cn } from "@/lib/utils";

const FIELD =
  "w-full min-w-0 rounded-md border bg-card px-3 text-base text-foreground transition-colors duration-150 placeholder:text-muted-foreground/80 hover:border-muted-foreground/40 aria-invalid:border-destructive";

function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return <input type={type} className={cn(FIELD, "h-8", className)} {...props} />;
}

function Textarea({ className, ...props }: React.ComponentProps<"textarea">) {
  return (
    <textarea className={cn(FIELD, "min-h-16 resize-y py-2 leading-5", className)} {...props} />
  );
}

export { Input, Textarea };
