import { useQuery } from "@tanstack/react-query";
import { createContext, type ReactNode, useCallback, useContext, useMemo, useState } from "react";
import type { Repo } from "@/api/client";
import { queries } from "@/lib/queries";

const KEY = "igloo.repo";

type SelectedRepo = { id: string | null; select: (id: string) => void };

const Context = createContext<SelectedRepo | null>(null);

/** Holds the repository the console shows, remembered across visits in `localStorage`. */
export function SelectedRepoProvider({ children }: { children: ReactNode }) {
  const [id, setId] = useState(() => localStorage.getItem(KEY));
  const select = useCallback((next: string) => {
    localStorage.setItem(KEY, next);
    setId(next);
  }, []);
  const value = useMemo(() => ({ id, select }), [id, select]);
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useSelectedRepo(): SelectedRepo {
  const value = useContext(Context);
  if (!value) throw new Error("useSelectedRepo needs a SelectedRepoProvider");
  return value;
}

/**
 * The repository the pages show: the selected one, or the first when none is selected or the
 * selection no longer exists. `pending` is true while the repositories load.
 */
export function useCurrentRepo(): { repo: Repo | null; pending: boolean; error: unknown } {
  const repos = useQuery(queries.repos());
  const selected = useSelectedRepo();
  const repo = repos.data?.find((repo) => repo.id === selected.id) ?? repos.data?.[0] ?? null;
  return { repo, pending: repos.isPending, error: repos.error };
}
