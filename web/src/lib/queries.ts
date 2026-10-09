import { queryOptions } from "@tanstack/react-query";
import { api } from "@/api/client";

/** Query definitions: the only place query keys are spelled. */
export const queries = {
  repos: () => queryOptions({ queryKey: ["repos"], queryFn: () => api.repos() }),
};
