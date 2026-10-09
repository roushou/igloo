import { Command as Cmdk } from "cmdk";
import type * as React from "react";
import { cn } from "@/lib/utils";

/** The searchable list inside the palette; filtering is cmdk's, over each item's label and keywords. */
function Command({ className, ...props }: React.ComponentProps<typeof Cmdk>) {
  return <Cmdk className={cn("flex flex-col", className)} {...props} />;
}

function CommandInput({ className, ...props }: React.ComponentProps<typeof Cmdk.Input>) {
  return (
    <Cmdk.Input
      autoFocus
      className={cn(
        "h-12 w-full bg-transparent text-lg outline-none placeholder:text-muted-foreground",
        className,
      )}
      {...props}
    />
  );
}

function CommandList({ className, ...props }: React.ComponentProps<typeof Cmdk.List>) {
  return <Cmdk.List className={cn("max-h-[22rem] overflow-y-auto p-1.5", className)} {...props} />;
}

function CommandGroup({ className, ...props }: React.ComponentProps<typeof Cmdk.Group>) {
  return (
    <Cmdk.Group
      className={cn(
        "[&_[cmdk-group-heading]]:caption [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:pt-2.5 [&_[cmdk-group-heading]]:pb-1",
        className,
      )}
      {...props}
    />
  );
}

function CommandItem({ className, ...props }: React.ComponentProps<typeof Cmdk.Item>) {
  return (
    <Cmdk.Item
      className={cn(
        "flex cursor-pointer items-center gap-2.5 rounded-md px-2 py-2 text-base select-none data-[disabled=true]:cursor-not-allowed data-[disabled=true]:opacity-50 data-[selected=true]:bg-accent [&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:text-muted-foreground",
        className,
      )}
      {...props}
    />
  );
}

function CommandEmpty(props: React.ComponentProps<typeof Cmdk.Empty>) {
  return (
    <Cmdk.Empty className="px-4 py-8 text-center text-base text-muted-foreground" {...props} />
  );
}

export { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList };
