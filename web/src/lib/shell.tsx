import { createContext, type ReactNode, useCallback, useContext, useMemo, useState } from "react";

const KEY = "igloo.sidebar";

type Shell = {
  /** The sidebar shows icons only. */
  collapsed: boolean;
  toggleCollapsed: () => void;
  /** The sidebar sheet of a narrow viewport is open. */
  sheetOpen: boolean;
  setSheetOpen: (open: boolean) => void;
};

const Context = createContext<Shell | null>(null);

/** Holds the state of the sidebar: collapsed or not (remembered in `localStorage`) and the sheet. */
export function ShellProvider({ children }: { children: ReactNode }) {
  const [collapsed, setCollapsed] = useState(() => localStorage.getItem(KEY) === "collapsed");
  const [sheetOpen, setSheetOpen] = useState(false);
  const toggleCollapsed = useCallback(() => {
    setCollapsed((value) => {
      localStorage.setItem(KEY, value ? "expanded" : "collapsed");
      return !value;
    });
  }, []);
  const value = useMemo(
    () => ({ collapsed, toggleCollapsed, sheetOpen, setSheetOpen }),
    [collapsed, toggleCollapsed, sheetOpen],
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useShell(): Shell {
  const value = useContext(Context);
  if (!value) throw new Error("useShell needs a ShellProvider");
  return value;
}
