import { QueryClient } from "@tanstack/react-query";
import { createRouter } from "@tanstack/react-router";
import { api } from "@/api/client";
import { routeTree } from "./routeTree.gen";

export type RouterContext = { queryClient: QueryClient };

export function getRouter() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { staleTime: 5_000, retry: false } },
  });
  const router = createRouter({
    routeTree,
    context: { queryClient },
    defaultPreload: "intent",
    scrollRestoration: true,
  });
  // A 401 anywhere returns to sign-in.
  api.handleUnauthorized(() => {
    queryClient.clear();
    void router.navigate({ to: "/signin" });
  });
  return router;
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof getRouter>;
  }
}
