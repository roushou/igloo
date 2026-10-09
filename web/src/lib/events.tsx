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
import { ActivityLog } from "./activity";
import { type ConnectionState, type EventNotice, EventStream } from "./event-stream";
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

type Events = { stream: EventStream; activity: ActivityLog };

const Context = createContext<Events | null>(null);

/**
 * Holds the tab's one event stream, open while mounted, and refetches the queries each event
 * makes stale. Needs a `QueryClientProvider`.
 */
export function EventsProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const [stale] = useState(() => invalidator(queryClient));
  const [events] = useState<Events>(() => {
    const activity = new ActivityLog();
    const stream = new EventStream({
      open: (lastId, signal) => api.events(lastId, signal),
      onNotice: (notice) => {
        activity.add(notice);
        stale.add(staleQueries(notice) as unknown[][]);
      },
    });
    return { stream, activity };
  });
  useEffect(() => {
    events.stream.start();
    return () => {
      events.stream.stop();
      stale.cancel();
    };
  }, [events, stale]);
  return <Context.Provider value={events}>{children}</Context.Provider>;
}

function useEvents(): Events {
  const events = useContext(Context);
  if (!events) throw new Error("events hooks need an EventsProvider");
  return events;
}

/** Whether the console is receiving events. */
export function useConnectionState(): ConnectionState {
  const { stream } = useEvents();
  return useSyncExternalStore(stream.subscribe, stream.getState, stream.getState);
}

/** The events received since the page opened, newest first. */
export function useRecentActivity(): readonly EventNotice[] {
  const { activity } = useEvents();
  return useSyncExternalStore(activity.subscribe, activity.getSnapshot, activity.getSnapshot);
}
