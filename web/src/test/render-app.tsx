import { QueryClient } from "@tanstack/react-query";
import { createMemoryHistory, createRouter, RouterProvider } from "@tanstack/react-router";
import { render } from "@testing-library/react";
import { vi } from "vitest";
import { api } from "@/api/client";
import { routeTree } from "@/routeTree.gen";

export const REPO = {
  id: "repo_01j9z3k4m5n6p7q8r9s0t1v2w3",
  location: "https://forge.example.com/acme/shop",
  default_branch: "main",
};

/** The event stream a stubbed server keeps open for the page. */
export class StubEvents {
  private readonly encoder = new TextEncoder();
  private controller: ReadableStreamDefaultController<Uint8Array> | null = null;
  /** The `Last-Event-ID` of each connection, in order; `null` for a first connection. */
  readonly connections: (string | null)[] = [];

  open(lastId: string | null): Response {
    this.connections.push(lastId);
    const body = new ReadableStream<Uint8Array>({
      start: (controller) => {
        this.controller = controller;
      },
    });
    return new Response(body, { headers: { "Content-Type": "text/event-stream" } });
  }

  /** Sends the notice of an event of `type` with `id`. */
  push(sequence: number, kind: string, type: string, id: string, repo?: string): void {
    const data = {
      sequence,
      kind,
      resource_type: type,
      resource_id: id,
      repo,
      time: "2026-01-01T00:00:00Z",
    };
    this.controller?.enqueue(
      this.encoder.encode(`id: ${sequence}\nevent: ${kind}\ndata: ${JSON.stringify(data)}\n\n`),
    );
  }

  /** Ends the current connection as a dropped network would. */
  drop(): void {
    this.controller?.close();
    this.controller = null;
  }
}

/**
 * Stubs the server: `GET /v1/repos` answers with `repos` for `token` and 401 otherwise, and
 * `GET /v1/events` streams what `events` is given.
 */
export function stubServer(token: string, repos: unknown[] = [REPO]) {
  const events = new StubEvents();
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init);
    const ok = request.headers.get("Authorization") === `Bearer ${token}`;
    if (!ok) return Response.json({ title: "Unauthorized", status: 401 }, { status: 401 });
    if (new URL(request.url).pathname === "/v1/events") {
      return events.open(request.headers.get("Last-Event-ID"));
    }
    return Response.json(repos);
  });
  vi.stubGlobal("fetch", fetchMock);
  return { fetchMock, events };
}

/** Renders the real route tree at `path`, with the API's 401 handler wired as in `getRouter`. */
export function renderApp(path: string) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const router = createRouter({
    routeTree,
    context: { queryClient },
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  api.handleUnauthorized(() => {
    queryClient.clear();
    void router.navigate({ to: "/signin" });
  });
  return { router, ...render(<RouterProvider router={router} />) };
}
