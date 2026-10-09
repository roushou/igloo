import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import {
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useState,
  useSyncExternalStore,
} from "react";
import { api } from "@/api/client";
import { type ConnectionState, EventStream } from "./event-stream";
import { staleQueries } from "./queries";

const FLUSH_MS = 50;

/** Invalidates the queries stale after the events of the last `FLUSH_MS`, each once. */
function invalidator(queryClient: QueryClient) {
  const pending = new Map<string, readonly unknown[]>();
  let timer: ReturnType<typeof setTimeout> | null = null;
  return {
    add(keys: readonly unknown[][]) {
      for (const key of keys) pending.set(JSON.stringify(key), key);
      timer ??= setTimeout(() => {
        timer = null;
        const keys = [...pending.values()];
        pending.clear();
        for (const queryKey of keys) void queryClient.invalidateQueries({ queryKey });
      }, FLUSH_MS);
    },
    cancel() {
      if (timer) clearTimeout(timer);
      timer = null;
      pending.clear();
    },
  };
}

const Context = createContext<EventStream | null>(null);

/**
 * Holds the tab's one event stream, open while mounted, and refetches the queries each event
 * makes stale. Needs a `QueryClientProvider`.
 */
export function EventsProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const [stale] = useState(() => invalidator(queryClient));
  const [stream] = useState(
    () =>
      new EventStream({
        open: (lastId, signal) => api.events(lastId, signal),
        onNotice: (notice) => stale.add(staleQueries(notice) as unknown[][]),
      }),
  );
  useEffect(() => {
    stream.start();
    return () => {
      stream.stop();
      stale.cancel();
    };
  }, [stream, stale]);
  return <Context.Provider value={stream}>{children}</Context.Provider>;
}

/** Whether the console is receiving events. */
export function useConnectionState(): ConnectionState {
  const stream = useContext(Context);
  if (!stream) throw new Error("useConnectionState needs an EventsProvider");
  return useSyncExternalStore(stream.subscribe, stream.getState, stream.getState);
}
