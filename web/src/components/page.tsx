import type { ReactNode } from "react";
import { ApiError } from "@/api/client";
import { cn } from "@/lib/utils";

/** A page: its title, optional actions beside it, and the content. */
export function Page({
  title,
  actions,
  meta,
  children,
}: {
  title: ReactNode;
  actions?: ReactNode;
  meta?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="mx-auto flex max-w-6xl flex-col gap-6">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h1 className="font-display text-2xl font-semibold tracking-tight">{title}</h1>
          {meta ? (
            <div className="mt-1 flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
              {meta}
            </div>
          ) : null}
        </div>
        {actions ? <div className="flex flex-wrap items-center gap-2">{actions}</div> : null}
      </header>
      {children}
    </section>
  );
}

/** A titled block of a page; `tone="needs-you"` marks what waits on the user in orange. */
export function Section({
  title,
  count,
  tone,
  children,
}: {
  title: string;
  count?: number;
  tone?: "needs-you";
  children: ReactNode;
}) {
  return (
    <section aria-label={title} className="flex flex-col gap-2">
      <h2
        className={cn(
          "flex items-center gap-2 text-sm font-semibold uppercase tracking-wide",
          tone === "needs-you" ? "text-expedition" : "text-muted-foreground",
        )}
      >
        {title}
        {count !== undefined ? (
          <span className="tabular rounded-full bg-muted px-1.5 text-xs font-medium">{count}</span>
        ) : null}
      </h2>
      {children}
    </section>
  );
}

/** A bordered list of rows. */
export function Rows({ children, label }: { children: ReactNode; label?: string }) {
  return (
    <ul
      aria-label={label}
      className="divide-y divide-border overflow-hidden rounded-lg border bg-card"
    >
      {children}
    </ul>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <p className="text-sm text-muted-foreground">{children}</p>;
}

export function Loading({ what }: { what: string }) {
  return (
    <p role="status" className="text-sm text-muted-foreground">
      Loading {what}…
    </p>
  );
}

/** A failed request: the server's problem when it gave one. */
export function ErrorNote({ error, what }: { error: unknown; what: string }) {
  const detail = error instanceof ApiError ? error.message : null;
  return (
    <p role="alert" className="text-sm text-failed">
      Could not load {what}.{detail ? ` ${detail}` : ""}
    </p>
  );
}

/** Wraps a query's states: loading, failure, or the data. */
export function Await<T>({
  query,
  what,
  children,
}: {
  query: { isPending: boolean; isError: boolean; error: unknown; data: T | undefined };
  what: string;
  children: (data: T) => ReactNode;
}) {
  if (query.isPending) return <Loading what={what} />;
  if (query.isError || query.data === undefined)
    return <ErrorNote error={query.error} what={what} />;
  return <>{children(query.data)}</>;
}

/** Label and value, for the facts under a title. */
export function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5">
      <span className="text-muted-foreground">{label}</span>
      {children}
    </span>
  );
}
