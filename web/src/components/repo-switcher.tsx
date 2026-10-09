import { useQuery } from "@tanstack/react-query";
import { Check, ChevronsUpDown, RefreshCw } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip } from "@/components/ui/tooltip";
import { format } from "@/lib/format";
import { queries } from "@/lib/queries";
import { useSelectedRepo } from "@/lib/selected-repo";
import { cn } from "@/lib/utils";

/** The initial of a repository, in a tile that stands for it when the rail is narrow. */
function RepoTile({ name }: { name: string }) {
  return (
    <span
      aria-hidden
      className="flex size-6 shrink-0 items-center justify-center rounded-md bg-foreground font-display text-sm font-semibold text-background"
    >
      {name.split("/").at(-1)?.charAt(0).toUpperCase()}
    </span>
  );
}

/** Lists the repositories and selects one; the first is selected until the user chooses. */
export function RepoSwitcher({ compact }: { compact?: boolean }) {
  const repos = useQuery(queries.repos());
  const selected = useSelectedRepo();

  if (repos.isPending) return <Skeleton className="h-9 w-full" />;
  if (repos.isError) {
    return (
      <button
        type="button"
        title="Repositories failed to load. Retry"
        onClick={() => void repos.refetch()}
        className={cn(
          "flex h-9 w-full items-center gap-2 rounded-md px-1.5 text-left text-base text-failed hover:bg-accent",
          compact && "justify-center",
        )}
      >
        <RefreshCw className="size-4 shrink-0" />
        {compact ? (
          <span className="sr-only">Retry loading repositories</span>
        ) : (
          "Retry repositories"
        )}
      </button>
    );
  }
  if (repos.data.length === 0) {
    return (
      <p
        className={cn(
          "flex h-9 items-center px-2 text-base text-muted-foreground",
          compact && "sr-only",
        )}
      >
        No repositories
      </p>
    );
  }
  const current = repos.data.find((repo) => repo.id === selected.id) ?? repos.data[0];
  const name = format.repoName(current?.location ?? "");
  return (
    <DropdownMenu>
      <Tooltip content={compact ? name : undefined} side="right">
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label="Repository"
            className={cn(
              "flex h-9 w-full items-center gap-2 rounded-md px-1.5 text-left text-base font-medium hover:bg-accent",
              compact && "justify-center",
            )}
          >
            <RepoTile name={name} />
            {compact ? null : (
              <>
                <span className="min-w-0 flex-1 truncate">{name}</span>
                <ChevronsUpDown className="size-3.5 shrink-0 text-muted-foreground" />
              </>
            )}
          </button>
        </DropdownMenuTrigger>
      </Tooltip>
      <DropdownMenuContent align="start" className="w-64">
        <DropdownMenuLabel>Repositories</DropdownMenuLabel>
        {repos.data.map((repo) => (
          <DropdownMenuItem key={repo.id} onSelect={() => selected.select(repo.id)}>
            <RepoTile name={format.repoName(repo.location)} />
            <span className="min-w-0 flex-1">
              <span className="block truncate">{format.repoName(repo.location)}</span>
              <span className="block truncate font-mono text-xs text-muted-foreground">
                {format.shortId(repo.id)}
              </span>
            </span>
            {repo.id === current?.id ? <Check className="text-foreground" /> : null}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
