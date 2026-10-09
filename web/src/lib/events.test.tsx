import { QueryClient } from "@tanstack/react-query";
import { screen, waitFor } from "@testing-library/react";
import { vi } from "vitest";
import { api } from "@/api/client";
import { renderApp, stubServer } from "@/test/render-app";
import { type EventNotice, EventStream } from "./event-stream";
import { queryKeys, staleQueries } from "./queries";

const TOKEN = "right-token";

function notice(type: string, id: string, repo?: string): EventNotice {
  return {
    sequence: 1,
    kind: `igloo.${type}.changed`,
    resource_type: type,
    resource_id: id,
    repo,
    time: "2026-01-01T00:00:00Z",
  };
}

describe("staleQueries", () => {
  it("maps an event to the resource and the lists it appears in", () => {
    expect(staleQueries(notice("task", "task_1", "repo_1"))).toEqual([
      queryKeys.task("task_1"),
      queryKeys.tasks("repo_1"),
    ]);
    expect(staleQueries(notice("run", "run_1", "repo_1"))).toEqual([
      queryKeys.run("run_1"),
      queryKeys.runs("repo_1"),
      queryKeys.changes("repo_1"),
    ]);
    expect(staleQueries(notice("job", "job_1"))).toEqual([queryKeys.job("job_1")]);
    expect(staleQueries(notice("seal", "seal_1"))).toEqual([]);
  });

  it("matches the lists of every repository when the event has none", () => {
    const client = new QueryClient();
    client.setQueryData(queryKeys.tasks("repo_1"), []);
    const [, list] = staleQueries(notice("task", "task_1"));
    client.invalidateQueries({ queryKey: list });
    expect(client.getQueryState(queryKeys.tasks("repo_1"))?.isInvalidated).toBe(true);
  });
});

/** A body that delivers `chunks` and then stays open until `end` is called. */
function body(chunks: string[]) {
  let controller!: ReadableStreamDefaultController<Uint8Array>;
  const stream = new ReadableStream<Uint8Array>({
    start(c) {
      controller = c;
      for (const chunk of chunks) c.enqueue(new TextEncoder().encode(chunk));
    },
  });
  return { stream, end: () => controller.close() };
}

const frame = (sequence: number) =>
  `id: ${sequence}\nevent: k\ndata: ${JSON.stringify({ ...notice("task", `task_${sequence}`), sequence })}\n\n`;

describe("EventStream", () => {
  afterEach(() => vi.useRealTimers());

  it("reconnects with backoff from the last id and reports its state", async () => {
    vi.useFakeTimers();
    const first = body([frame(1), frame(2)]);
    const second = body([frame(3)]);
    const opened: (string | null)[] = [];
    const bodies = [first, second];
    const seen: number[] = [];
    const stream = new EventStream({
      open: async (lastId) => {
        opened.push(lastId);
        const next = bodies.shift();
        if (!next) throw new Error("down");
        return next.stream;
      },
      onNotice: (n) => seen.push(n.sequence),
      initialDelayMs: 1_000,
      maxDelayMs: 4_000,
    });
    const states: string[] = [];
    stream.subscribe(() => states.push(stream.getState()));
    stream.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(stream.getState()).toBe("live");
    expect(seen).toEqual([1, 2]);

    first.end();
    await vi.advanceTimersByTimeAsync(0);
    expect(stream.getState()).toBe("reconnecting");
    await vi.advanceTimersByTimeAsync(1_000);
    expect(stream.getState()).toBe("live");
    expect(opened).toEqual([null, "2"]);
    expect(seen).toEqual([1, 2, 3]);

    second.end();
    await vi.advanceTimersByTimeAsync(0);
    // Attempts at +1s, +2s and +4s fail: offline from the third failure.
    await vi.advanceTimersByTimeAsync(1_000);
    expect(stream.getState()).toBe("reconnecting");
    await vi.advanceTimersByTimeAsync(2_000);
    expect(stream.getState()).toBe("offline");
    expect(opened.slice(2)).toEqual(["3", "3"]);
    stream.stop();
    expect(states).toContain("offline");
  });

  it("stops reading after stop", async () => {
    const open = vi.fn(async (_id: string | null, signal: AbortSignal) => {
      const { stream } = body([]);
      signal.addEventListener("abort", () => undefined);
      return stream;
    });
    const stream = new EventStream({ open, onNotice: () => undefined });
    stream.start();
    stream.stop();
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(open.mock.calls.length).toBeLessThanOrEqual(1);
  });
});

describe("the shell", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("shows the stream is live and refetches what an event changed", async () => {
    api.tokens.set(TOKEN);
    const { fetchMock, events } = stubServer(TOKEN);
    renderApp("/");
    expect(await screen.findByRole("status")).toHaveTextContent("Live");
    expect(events.connections).toEqual([null]);

    const reposCalls = () =>
      fetchMock.mock.calls.filter(([input]) => new Request(input).url.endsWith("/v1/repos")).length;
    const before = reposCalls();
    events.push(1, "igloo.repo.registered", "repo", "repo_x");
    await waitFor(() => expect(reposCalls()).toBeGreaterThan(before));
  });

  it("reconnects from the last event id after the connection drops", async () => {
    api.tokens.set(TOKEN);
    const { events } = stubServer(TOKEN);
    renderApp("/");
    await screen.findByText("Live");
    events.push(7, "igloo.task.created", "task", "task_1", "repo_1");
    events.drop();
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Reconnecting"));
    await waitFor(() => expect(events.connections).toEqual([null, "7"]), { timeout: 3_000 });
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Live"));
  });
});
