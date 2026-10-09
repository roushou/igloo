import type { ReactNode } from "react";
import { PageHeader } from "@/components/page";
import { cn } from "@/lib/utils";

/**
 * The list half of a list-and-detail page: its own header, filters and scrolling rows. Beside an
 * open detail it is a fixed-width column; alone it fills the page.
 */
export function ListPane({
  title,
  actions,
  toolbar,
  narrow,
  children,
}: {
  title: string;
  actions?: ReactNode;
  toolbar?: ReactNode;
  /** A detail is open beside the list. */
  narrow: boolean;
  children: ReactNode;
}) {
  return (
    <div
      className={cn(
        "@container flex h-full min-h-0 min-w-0 flex-col",
        narrow ? "w-[22rem] shrink-0 border-r xl:w-[26rem]" : "flex-1",
      )}
    >
      <PageHeader crumbs={[{ label: title }]} actions={actions} />
      {toolbar ? (
        <div className="border-b">
          <div className={cn("px-4 py-2.5", !narrow && "mx-auto w-full max-w-5xl")}>{toolbar}</div>
        </div>
      ) : null}
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div
          className={cn(
            "flex flex-col gap-6 py-4 [&_[data-section-head]]:px-4",
            !narrow && "mx-auto w-full max-w-5xl sm:px-4 sm:[&_[data-section-head]]:px-0",
          )}
        >
          {children}
        </div>
      </div>
    </div>
  );
}
