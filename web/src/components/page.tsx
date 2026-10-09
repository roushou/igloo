import { Link, type LinkProps } from "@tanstack/react-router";
import { ChevronRight, CircleAlert, Menu, RefreshCw } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { Fragment, type ReactNode } from "react";
import { ApiError } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { useIsWide } from "@/lib/media";
import { useMotion } from "@/lib/motion";
import { useShell } from "@/lib/shell";
import { cn } from "@/lib/utils";

/** One step of the breadcrumbs: a link, or plain text for the page itself. */
export type Crumb = { label: ReactNode; link?: Pick<LinkProps, "to" | "params" | "search"> };

/**
 * The bar at the top of a page: the menu button on a narrow viewport, the breadcrumbs, and the
 * page's actions on the right. It never scrolls away: the page below it does.
 */
export function PageHeader({ crumbs, actions }: { crumbs: Crumb[]; actions?: ReactNode }) {
  const shell = useShell();
  const wide = useIsWide();
  return (
    <header className="flex h-12 shrink-0 items-center gap-2 border-b px-3 sm:px-4">
      {wide ? null : (
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Open the menu"
          onClick={() => shell.setSheetOpen(true)}
        >
          <Menu />
        </Button>
      )}
      <nav aria-label="Breadcrumb" className="min-w-0 flex-1">
        <ol className="flex items-center gap-1 text-base">
          {crumbs.map((crumb, index) => {
            const last = index === crumbs.length - 1;
            return (
              // biome-ignore lint/suspicious/noArrayIndexKey: crumbs are a fixed trail, identified by their place.
              <Fragment key={index}>
                {index > 0 ? (
                  <ChevronRight
                    aria-hidden
                    className="size-3.5 shrink-0 text-muted-foreground/60"
                  />
                ) : null}
                <li className={cn("min-w-0", last ? "truncate font-medium" : "shrink-0")}>
                  {crumb.link && !last ? (
                    <Link
                      {...crumb.link}
                      className="rounded text-muted-foreground hover:text-foreground"
                    >
                      {crumb.label}
                    </Link>
                  ) : (
                    <span aria-current={last ? "page" : undefined}>{crumb.label}</span>
                  )}
                </li>
              </Fragment>
            );
          })}
        </ol>
      </nav>
      {actions ? <div className="flex shrink-0 items-center gap-1.5">{actions}</div> : null}
    </header>
  );
}

/**
 * A page: a sticky header with breadcrumbs and actions, then a scrolling body with the title,
 * the facts under it, and the content.
 */
export function Page({
  crumbs,
  title,
  meta,
  actions,
  wide,
  compactTitle,
  children,
}: {
  crumbs: Crumb[];
  title: ReactNode;
  actions?: ReactNode;
  meta?: ReactNode;
  /** Let the content use the whole width, as a diff does. */
  wide?: boolean;
  /** A smaller title, for a title that is a sentence. */
  compactTitle?: boolean;
  children: ReactNode;
}) {
  const { transition } = useMotion();
  return (
    <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
      <PageHeader crumbs={crumbs} actions={actions} />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <motion.section
          initial={{ opacity: 0, y: 6 }}
          animate={{ opacity: 1, y: 0 }}
          transition={transition(0.22)}
          className={cn(
            "@container mx-auto flex w-full flex-col gap-6 px-4 py-6 sm:px-8",
            wide ? "max-w-none" : "max-w-5xl",
          )}
        >
          <div className="min-w-0">
            <h1
              className={cn(
                "font-semibold tracking-tight text-balance",
                compactTitle ? "text-xl" : "text-2xl",
              )}
            >
              {title}
            </h1>
            {meta ? (
              <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-2 text-base text-muted-foreground">
                {meta}
              </div>
            ) : null}
          </div>
          {children}
        </motion.section>
      </div>
    </div>
  );
}

/** A titled block of a page; `tone="needs-you"` marks what waits on the user in orange. */
export function Section({
  title,
  count,
  tone,
  action,
  children,
}: {
  title: string;
  count?: number;
  tone?: "needs-you";
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section aria-label={title} className="flex flex-col gap-1">
      <div data-section-head className="flex h-8 items-center gap-2">
        <h2
          className={cn(
            "flex items-center gap-2 font-sans text-base font-semibold",
            tone === "needs-you" && "text-expedition",
          )}
        >
          {tone === "needs-you" ? (
            <span aria-hidden className="size-1.5 rounded-full bg-expedition" />
          ) : null}
          {title}
        </h2>
        {count !== undefined ? (
          <span className="tabular text-sm text-muted-foreground">{count}</span>
        ) : null}
        {action ? <div className="ml-auto">{action}</div> : null}
      </div>
      {children}
    </section>
  );
}

/** A list of rows divided by hairlines; rows that come and go slide in and out. */
export function Rows({ children, label }: { children: ReactNode; label?: string }) {
  return (
    <ul aria-label={label} className="divide-y divide-border border-y">
      <AnimatePresence initial={false}>{children}</AnimatePresence>
    </ul>
  );
}

/** Placeholder rows for a list that loads. */
export function SkeletonRows({ rows = 4 }: { rows?: number }) {
  return (
    <div aria-hidden className="divide-y divide-border border-y">
      {Array.from({ length: rows }, (_, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: identical placeholders have no identity.
        <div key={index} className="flex items-center gap-3 px-4 py-3">
          <Skeleton className="h-5 w-20 rounded-full" />
          <Skeleton className="h-4 flex-1" />
          <Skeleton className="hidden h-4 w-20 sm:block" />
        </div>
      ))}
    </div>
  );
}

/** A quiet line for a list that is empty and needs no explanation. */
export function Empty({ children }: { children: ReactNode }) {
  return <p className="py-2 text-base text-muted-foreground">{children}</p>;
}

/** What a page shows while its data loads: placeholder rows, and a status for assistive technology. */
export function Loading({ what, rows }: { what: string; rows?: number }) {
  return (
    <div>
      <p role="status" className="sr-only">
        Loading {what}…
      </p>
      <SkeletonRows rows={rows} />
    </div>
  );
}

/** A failed request: what failed, the server's reason when it gave one, and a way to retry. */
export function ErrorNote({
  error,
  what,
  onRetry,
}: {
  error: unknown;
  what: string;
  onRetry?: () => unknown;
}) {
  const detail = error instanceof ApiError ? error.message : null;
  return (
    <div
      role="alert"
      className="flex items-start gap-3 rounded-lg border border-failed/30 bg-failed-soft p-4 text-base"
    >
      <CircleAlert className="mt-0.5 size-4 shrink-0 text-failed" />
      <div className="min-w-0 flex-1">
        <p className="font-medium text-failed">Could not load {what}.</p>
        {detail ? <p className="mt-0.5 text-muted-foreground">{detail}</p> : null}
      </div>
      {onRetry ? (
        <Button variant="outline" size="sm" onClick={() => void onRetry()}>
          <RefreshCw className="size-3.5" />
          Retry
        </Button>
      ) : null}
    </div>
  );
}

/** Wraps a query's states: loading, failure with a retry, or the data. */
export function Await<T>({
  query,
  what,
  rows,
  children,
}: {
  query: {
    isPending: boolean;
    isError: boolean;
    error: unknown;
    data: T | undefined;
    refetch?: () => unknown;
  };
  what: string;
  rows?: number;
  children: (data: T) => ReactNode;
}) {
  if (query.isPending) return <Loading what={what} rows={rows} />;
  if (query.isError || query.data === undefined) {
    return <ErrorNote error={query.error} what={what} onRetry={query.refetch} />;
  }
  return <>{children(query.data)}</>;
}

/** Label and value, for the facts under a title. */
export function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5">
      <span className="text-muted-foreground">{label}</span>
      <span className="text-foreground">{children}</span>
    </span>
  );
}
