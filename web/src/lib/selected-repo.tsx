import { createContext, type ReactNode, useCallback, useContext, useMemo, useState } from "react";

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
