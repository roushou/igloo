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

/** Stubs the server: `GET /v1/repos` answers with `repos` for `token` and 401 otherwise. */
export function stubServer(token: string, repos: unknown[] = [REPO]) {
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init);
    const ok = request.headers.get("Authorization") === `Bearer ${token}`;
    return ok
      ? Response.json(repos)
      : Response.json({ title: "Unauthorized", status: 401 }, { status: 401 });
  });
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
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
