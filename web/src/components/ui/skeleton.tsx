import { cn } from "@/lib/utils";

/** A grey block that stands where content will load. Decorative: pair it with a status text. */
export function Skeleton({ className }: { className?: string }) {
  return <div aria-hidden className={cn("skeleton rounded-md", className)} />;
}
