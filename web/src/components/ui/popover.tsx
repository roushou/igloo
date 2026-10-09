import { Popover as Radix } from "radix-ui";
import type * as React from "react";
import { cn } from "@/lib/utils";

const Popover = Radix.Root;
const PopoverTrigger = Radix.Trigger;

function PopoverContent({
  className,
  align = "end",
  sideOffset = 6,
  ...props
}: React.ComponentProps<typeof Radix.Content>) {
  return (
    <Radix.Portal>
      <Radix.Content
        align={align}
        sideOffset={sideOffset}
        collisionPadding={8}
        className={cn(
          "z-50 w-96 max-w-[calc(100vw-1rem)] rounded-lg border bg-card p-3 text-base shadow-float data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
          className,
        )}
        {...props}
      />
    </Radix.Portal>
  );
}

export { Popover, PopoverContent, PopoverTrigger };
