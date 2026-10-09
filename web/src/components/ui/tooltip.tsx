import { Tooltip as Radix } from "radix-ui";
import type { ReactNode } from "react";

/** Wraps the app once so tooltips share their delay. */
export function TooltipProvider({ children }: { children: ReactNode }) {
  return (
    <Radix.Provider delayDuration={350} skipDelayDuration={150}>
      {children}
    </Radix.Provider>
  );
}

/**
 * A label that appears when `children` is hovered or focused. Without `content` it shows nothing,
 * and `children` keep their place in the tree whether or not there is content to show.
 */
export function Tooltip({
  content,
  children,
  side = "top",
}: {
  content?: ReactNode;
  children: ReactNode;
  side?: "top" | "right" | "bottom" | "left";
}) {
  return (
    <Radix.Root open={content ? undefined : false}>
      <Radix.Trigger asChild>{children}</Radix.Trigger>
      {content ? (
        <Radix.Portal>
          <Radix.Content
            side={side}
            sideOffset={6}
            className="z-[60] max-w-72 rounded-md bg-foreground px-2 py-1 text-sm text-background shadow-float data-[state=delayed-open]:animate-in data-[state=delayed-open]:fade-in-0 data-[state=delayed-open]:zoom-in-95"
          >
            {content}
          </Radix.Content>
        </Radix.Portal>
      ) : null}
    </Radix.Root>
  );
}
