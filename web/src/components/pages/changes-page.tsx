import { useQuery } from "@tanstack/react-query";
import { getRouteApi, Outlet, useMatch, useNavigate } from "@tanstack/react-router";
import { ArrowRight, GitPullRequest } from "lucide-react";
import type { Change } from "@/api/client";
import { EmptyState } from "@/components/empty-state";
import { FilterBar } from "@/components/filter-bar";
import { ItemRow } from "@/components/item-row";
import { ListPane } from "@/components/list-pane";
import { Await, Empty, Loading, Rows, Section } from "@/components/page";
import { RelativeTime } from "@/components/relative-time";
import { WithRepo } from "@/components/repo-page";
import { ShortId } from "@/components/short-id";
import { matchesQuery } from "@/lib/list-search";
import { useIsWide } from "@/lib/media";
import { queries } from "@/lib/queries";
import { waitingOn } from "@/lib/readiness";
import { STATE_LABELS, STATES, type State, status } from "@/lib/status";

const route = getRouteApi("/_app/changes");

/**
 * Changes: every change of the repository, grouped by state with what needs the user first. A
 * change opens beside the list on a wide viewport, and in its place on a narrow one.
 */
export function ChangesLayout() {
  const open = useMatch({ from: "/_app/changes/$id", shouldThrow: false });
  const wide = useIsWide();
  const detail = Boolean(open);
  return (
    <div className="flex h-full min-h-0">
      {detail && !wide ? null : (
        <WithRepo
          placeholder={
            <ListPane title="Changes" narrow={detail}>
              <Loading what="changes" />
            </ListPane>
          }
        >
          {(repo) => <ChangeList repo={repo.id} narrow={detail} active={open?.params.id} />}
        </WithRepo>
      )}
      {detail ? (
        <div className="min-w-0 flex-1">
          <Outlet />
        </div>
      ) : null}
    </div>
  );
}

function ChangeList({ repo, narrow, active }: { repo: string; narrow: boolean; active?: string }) {
  const search = route.useSearch();
  const navigate = useNavigate();
  const changes = useQuery(queries.changes(repo));
  const data = changes.data ?? [];
  const counts: Partial<Record<State, number>> = {};
  for (const change of data)
    counts[status.change(change)] = (counts[status.change(change)] ?? 0) + 1;

  const setSearch = (patch: { state?: State | undefined; q?: string }, replace = false) =>
    void navigate({ to: "/changes", search: (prev) => ({ ...prev, ...patch }), replace });

  return (
    <ListPane
      title="Changes"
      narrow={narrow}
      toolbar={
        data.length > 0 ? (
          <FilterBar
            what="changes"
            counts={counts}
            state={search.state}
            onState={(state) => setSearch({ state })}
            query={search.q ?? ""}
            onQuery={(q) => setSearch({ q: q || undefined }, true)}
          />
        ) : null
      }
    >
      <Await query={changes} what="changes">
        {(changes) =>
          changes.length === 0 ? (
            <EmptyState
              icon={GitPullRequest}
              title="No changes yet"
              command={`igloo change push <change> --repo ${repo}`}
            >
              No change yet. A task's work, or a pushed branch, opens one; its checks, comments and
              merge readiness appear here.
            </EmptyState>
          ) : (
            <Groups changes={changes} state={search.state} query={search.q} active={active} />
          )
        }
      </Await>
    </ListPane>
  );
}

function Groups({
  changes,
  state,
  query,
  active,
}: {
  changes: Change[];
  state?: State;
  query?: string;
  active?: string;
}) {
  const visible = changes.filter(
    (change) =>
      (!state || status.change(change) === state) &&
      matchesQuery(query, change.title, change.source_branch, change.target_branch, change.id),
  );
  if (visible.length === 0) return <Empty>No change matches the filter.</Empty>;
  return (
    <>
      {STATES.map((group) => {
        const rows = visible.filter((change) => status.change(change) === group);
        return rows.length === 0 ? null : (
          <Section
            key={group}
            title={STATE_LABELS[group]}
            count={rows.length}
            tone={group === "needs-you" ? "needs-you" : undefined}
          >
            <Rows label={STATE_LABELS[group]}>
              {rows.map((change) => (
                <ChangeRow key={change.id} change={change} selected={change.id === active} />
              ))}
            </Rows>
          </Section>
        );
      })}
    </>
  );
}

function ChangeRow({ change, selected }: { change: Change; selected: boolean }) {
  const state = status.change(change);
  const latest = change.revisions.at(-1);
  return (
    <ItemRow
      state={state}
      title={change.title}
      link={{ to: "/changes/$id", params: { id: change.id }, search: true }}
      selected={selected}
      details={
        <>
          <span className="inline-flex items-center gap-1 font-mono">
            {change.source_branch}
            <ArrowRight className="size-3" />
            {change.target_branch}
          </span>
          {state === "needs-you" ? (
            <span className="text-expedition">{waitingOn(change)}</span>
          ) : null}
        </>
      }
      meta={
        <>
          <span title={`${change.revisions.length} revisions`}>r{change.revisions.length}</span>
          <ShortId id={change.id} />
        </>
      }
      time={latest ? <RelativeTime at={latest.created_at} /> : undefined}
    />
  );
}
