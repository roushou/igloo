import { useNavigate } from "@tanstack/react-router";
import {
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useShell } from "@/lib/shell";

/** One shortcut for the help dialog: the keys to press and what they do. */
export type Shortcut = { keys: string[]; sequence?: boolean; label: string };

/** Every keyboard shortcut of the console, in the order the help dialog lists them. */
export const SHORTCUTS: { group: string; items: Shortcut[] }[] = [
  {
    group: "Anywhere",
    items: [
      { keys: ["⌘", "K"], label: "Search, open a resource by id, run a command" },
      { keys: ["?"], label: "Show the shortcuts" },
      { keys: ["["], label: "Collapse or expand the sidebar" },
      { keys: ["C"], label: "Create a task" },
    ],
  },
  {
    group: "Go to",
    items: [
      { keys: ["G", "N"], sequence: true, label: "Now" },
      { keys: ["G", "T"], sequence: true, label: "Tasks" },
      { keys: ["G", "C"], sequence: true, label: "Changes" },
      { keys: ["G", "W"], sequence: true, label: "Workspaces" },
      { keys: ["G", "S"], sequence: true, label: "System" },
      { keys: ["G", "R"], sequence: true, label: "Repository" },
    ],
  },
  {
    group: "Lists",
    items: [
      { keys: ["J"], label: "Next row" },
      { keys: ["K"], label: "Previous row" },
      { keys: ["Enter"], label: "Open the row" },
    ],
  },
];

/** The pages `g` then a key goes to. */
const GO_TO = {
  n: "/",
  t: "/tasks",
  c: "/changes",
  w: "/workspaces",
  s: "/system",
  r: "/settings",
} as const;

/** How long after `g` the second key is still taken as part of the sequence. */
const SEQUENCE_MS = 1_200;

type Shortcuts = {
  paletteOpen: boolean;
  setPaletteOpen: (open: boolean) => void;
  helpOpen: boolean;
  setHelpOpen: (open: boolean) => void;
  openPalette: () => void;
  openHelp: () => void;
};

const Context = createContext<Shortcuts | null>(null);

/** Whether the key press happens where the user is typing, so shortcuts must stay out of it. */
function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName) ||
    target.closest("[role=dialog]") !== null
  );
}

/** Moves the keyboard focus to the next (`by` = 1) or previous (-1) list row on the page. */
function moveRow(by: 1 | -1): void {
  const rows = [...document.querySelectorAll<HTMLElement>("[data-nav-row]")].filter(
    (row) => row.getClientRects().length > 0,
  );
  if (rows.length === 0) return;
  const at = rows.indexOf(document.activeElement as HTMLElement);
  const next = at === -1 ? (by === 1 ? 0 : rows.length - 1) : (at + by + rows.length) % rows.length;
  rows[next]?.focus();
}

/**
 * The console's keyboard: ⌘K, `?`, `g` then a page letter, `j` and `k` through rows, `c` to
 * create a task and `[` for the sidebar. Keys typed into a field or a dialog are left alone.
 */
export function ShortcutsProvider({ children }: { children: ReactNode }) {
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const navigate = useNavigate();
  const shell = useShell();
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const clear = () => {
      if (pending.current) clearTimeout(pending.current);
      pending.current = null;
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() === "k" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        setPaletteOpen((value) => !value);
        return;
      }
      if (event.metaKey || event.ctrlKey || event.altKey || isTyping(event.target)) return;
      const key = event.key.toLowerCase();
      if (pending.current) {
        clear();
        const to = GO_TO[key as keyof typeof GO_TO];
        if (to) {
          event.preventDefault();
          void navigate({ to });
        }
        return;
      }
      switch (key) {
        case "g":
          pending.current = setTimeout(clear, SEQUENCE_MS);
          break;
        case "?":
          event.preventDefault();
          setHelpOpen(true);
          break;
        case "j":
          moveRow(1);
          break;
        case "k":
          moveRow(-1);
          break;
        case "c":
          event.preventDefault();
          void navigate({ to: "/tasks", search: { new: true } });
          break;
        case "[":
          shell.toggleCollapsed();
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      clear();
    };
  }, [navigate, shell]);

  const value = useMemo<Shortcuts>(
    () => ({
      paletteOpen,
      setPaletteOpen,
      helpOpen,
      setHelpOpen,
      openPalette: () => setPaletteOpen(true),
      openHelp: () => setHelpOpen(true),
    }),
    [paletteOpen, helpOpen],
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useShortcuts(): Shortcuts {
  const value = useContext(Context);
  if (!value) throw new Error("useShortcuts needs a ShortcutsProvider");
  return value;
}
