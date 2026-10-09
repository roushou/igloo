import type { LucideIcon } from "lucide-react";
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useId,
  useMemo,
  useState,
} from "react";

/** One thing the command palette can do. */
export type Command = {
  id: string;
  label: string;
  /** The palette heading it is listed under. */
  group: string;
  icon?: LucideIcon;
  /** Extra words the palette matches besides the label. */
  keywords?: string[];
  /** The shortcut that does the same, shown beside it. */
  shortcut?: string;
  /** Why it cannot run now; it is listed, greyed, with this reason. */
  unmet?: string | null;
  run: () => void;
};

type Registry = {
  commands: Command[];
  register: (owner: string, commands: Command[]) => void;
  unregister: (owner: string) => void;
};

const Context = createContext<Registry | null>(null);

/** Holds the commands that pages offer while they are on screen. */
export function CommandsProvider({ children }: { children: ReactNode }) {
  const [owned, setOwned] = useState<Record<string, Command[]>>({});
  const register = useCallback(
    (owner: string, commands: Command[]) => setOwned((all) => ({ ...all, [owner]: commands })),
    [],
  );
  const unregister = useCallback(
    (owner: string) =>
      setOwned((all) => {
        const { [owner]: _gone, ...rest } = all;
        return rest;
      }),
    [],
  );
  const value = useMemo<Registry>(
    () => ({ commands: Object.values(owned).flat(), register, unregister }),
    [owned, register, unregister],
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

/** The commands pages currently offer. */
export function usePageCommands(): Command[] {
  const registry = useContext(Context);
  if (!registry) throw new Error("usePageCommands needs a CommandsProvider");
  return registry.commands;
}

/**
 * Offers `commands` in the palette while the calling page is mounted. Pass a memoised list, so
 * the registry changes only when the commands do.
 */
export function useRegisterCommands(commands: Command[]): void {
  const registry = useContext(Context);
  if (!registry) throw new Error("useRegisterCommands needs a CommandsProvider");
  const { register, unregister } = registry;
  const owner = useId();
  useEffect(() => {
    register(owner, commands);
    return () => unregister(owner);
  }, [owner, commands, register, unregister]);
}
