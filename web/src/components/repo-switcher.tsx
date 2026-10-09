import { useQuery } from "@tanstack/react-query";
import { queries } from "@/lib/queries";
import { useSelectedRepo } from "@/lib/selected-repo";

/** Lists the repositories and selects one; the first is selected until the user chooses. */
export function RepoSwitcher() {
  const repos = useQuery(queries.repos());
  const selected = useSelectedRepo();

  if (repos.isPending) return <p className="px-2 text-sm text-muted-foreground">Loading…</p>;
  if (repos.isError)
    return <p className="px-2 text-sm text-destructive">Repositories failed to load</p>;
  if (repos.data.length === 0) {
    return <p className="px-2 text-sm text-muted-foreground">No repositories</p>;
  }
  const current = repos.data.find((repo) => repo.id === selected.id) ?? repos.data[0];
  return (
    <select
      aria-label="Repository"
      className="h-9 w-full rounded-md border bg-transparent px-2 text-sm"
      value={current?.id}
      onChange={(event) => selected.select(event.target.value)}
    >
      {repos.data.map((repo) => (
        <option key={repo.id} value={repo.id}>
          {repo.location}
        </option>
      ))}
    </select>
  );
}
