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

/** A server-sent event stream a stubbed server keeps open for the page. */
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

  /** Sends `text` as is, such as a job's `stdout` event. */
  raw(text: string): void {
    this.controller?.enqueue(this.encoder.encode(text));
  }

  /** Ends the current connection as a dropped network would. */
  drop(): void {
    this.controller?.close();
    this.controller = null;
  }
}

/** What a stubbed route answers: JSON for a 200, or a `Response` for anything else. */
export type StubRoute = unknown | ((request: Request, body: unknown) => unknown | Promise<unknown>);

/** A request the stubbed server received. */
export type StubCall = { method: string; path: string; search: string; body: unknown };

/**
 * Stubs the server: `GET /v1/repos` answers with `repos` for `token` and 401 otherwise,
 * `GET /v1/events` streams what `events` is given, and `routes` answers the rest by
 * `"METHOD /path"`. A route is a recorded response body, or a function returning one or a
 * `Response`. Unknown routes answer 404.
 */
export function stubServer(
  token: string,
  repos: unknown[] = [REPO],
  routes: Record<string, StubRoute> = {},
) {
  const events = new StubEvents();
  const calls: StubCall[] = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init);
    const ok = request.headers.get("Authorization") === `Bearer ${token}`;
    if (!ok) return Response.json({ title: "Unauthorized", status: 401 }, { status: 401 });
    const url = new URL(request.url);
    if (url.pathname === "/v1/events") {
      return events.open(request.headers.get("Last-Event-ID"));
    }
    const text = request.method === "GET" ? "" : await request.clone().text();
    let body: unknown = text;
    try {
      body = text ? JSON.parse(text) : undefined;
    } catch {
      // Not JSON: the secret endpoint takes plain text.
    }
    calls.push({ method: request.method, path: url.pathname, search: url.search, body });
    if (request.method === "GET" && url.pathname === "/v1/repos") return Response.json(repos);
    const route = routes[`${request.method} ${url.pathname}`];
    if (route === undefined) {
      return Response.json(
        { title: "Not found", status: 404, code: "stub.not_found", type: "about:blank" },
        { status: 404 },
      );
    }
    const answer = typeof route === "function" ? await route(request, body) : route;
    return answer instanceof Response ? answer : Response.json(answer);
  });
  vi.stubGlobal("fetch", fetchMock);
  return { fetchMock, events, calls };
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
