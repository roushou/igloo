import { Command as Cmdk } from "cmdk";
import type * as React from "react";
import { cn } from "@/lib/utils";

function CommandDialog({
  open,
  onOpenChange,
  title,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  children: React.ReactNode;
}) {
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50">
      <button
        type="button"
        tabIndex={-1}
        aria-label="Close"
        className="absolute inset-0 bg-black/50"
        onClick={() => onOpenChange(false)}
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className="absolute top-[20%] left-1/2 w-full max-w-lg -translate-x-1/2 overflow-hidden rounded-lg border bg-card shadow-lg"
        onKeyDown={(event) => {
          if (event.key === "Escape") onOpenChange(false);
        }}
      >
        <Cmdk shouldFilter={false} label={title} className="flex flex-col">
          {children}
        </Cmdk>
      </div>
    </div>
  );
}

function CommandInput({ className, ...props }: React.ComponentProps<typeof Cmdk.Input>) {
  return (
    <Cmdk.Input
      autoFocus
      className={cn(
        "h-12 w-full border-b bg-transparent px-4 text-sm outline-none placeholder:text-muted-foreground",
        className,
      )}
      {...props}
    />
  );
}

function CommandList(props: React.ComponentProps<typeof Cmdk.List>) {
  return <Cmdk.List className="max-h-72 overflow-y-auto p-1" {...props} />;
}

function CommandItem({ className, ...props }: React.ComponentProps<typeof Cmdk.Item>) {
  return (
    <Cmdk.Item
      className={cn(
        "flex cursor-pointer items-center gap-2 rounded-sm px-3 py-2 text-sm data-[selected=true]:bg-accent data-[selected=true]:text-accent-foreground",
        className,
      )}
      {...props}
    />
  );
}

function CommandEmpty(props: React.ComponentProps<typeof Cmdk.Empty>) {
  return <Cmdk.Empty className="px-4 py-6 text-center text-sm text-muted-foreground" {...props} />;
}

export { CommandDialog, CommandEmpty, CommandInput, CommandItem, CommandList };
