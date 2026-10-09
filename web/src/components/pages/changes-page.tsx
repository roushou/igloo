import { useQuery } from "@tanstack/react-query";
import type { Change } from "@/api/client";
import { ItemRow } from "@/components/item-row";
import { Await, Empty, Page, Rows, Section } from "@/components/page";
import { WithRepo } from "@/components/repo-page";
import { ShortId } from "@/components/short-id";
import { queries } from "@/lib/queries";
import { waitingOn } from "@/lib/readiness";
import { STATE_LABELS, STATES, status } from "@/lib/status";

/** Changes: every change of the repository, grouped by state, what needs the user first. */
export function ChangesPage() {
  return (
    <Page title="Changes">
      <WithRepo>{(repo) => <Changes repo={repo.id} />}</WithRepo>
    </Page>
  );
}

function Changes({ repo }: { repo: string }) {
  const changes = useQuery(queries.changes(repo));
  return (
    <Await query={changes} what="changes">
      {(changes) =>
        changes.length === 0 ? (
          <Empty>No change yet. A task's work, or a pushed branch, opens one.</Empty>
        ) : (
          <div className="flex flex-col gap-8">
            {STATES.map((state) => {
              const group = changes.filter((change) => status.change(change) === state);
              return group.length === 0 ? null : (
                <Section
                  key={state}
                  title={STATE_LABELS[state]}
                  count={group.length}
                  tone={state === "needs-you" ? "needs-you" : undefined}
                >
                  <Rows label={STATE_LABELS[state]}>
                    {group.map((change) => (
                      <ChangeRow key={change.id} change={change} />
                    ))}
                  </Rows>
                </Section>
              );
            })}
          </div>
        )
      }
    </Await>
  );
}

function ChangeRow({ change }: { change: Change }) {
  const state = status.change(change);
  return (
    <ItemRow
      state={state}
      title={change.title}
      link={{ to: "/changes/$id", params: { id: change.id } }}
      details={
        <>
          <span className="font-mono">
            {change.source_branch} → {change.target_branch}
          </span>
          {state === "needs-you" ? <span>{waitingOn(change)}</span> : null}
          <ShortId id={change.id} />
        </>
      }
      aside={`${change.revisions.length} revision${change.revisions.length === 1 ? "" : "s"}`}
    />
  );
}
