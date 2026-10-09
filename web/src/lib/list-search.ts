import { STATES, type State } from "@/lib/status";

/** What a list page keeps in its URL: the state filter, the text filter, and the create dialog. */
export type ListSearch = { state?: State; q?: string; new?: boolean };

/** Reads a list page's search parameters, dropping what is not valid. */
export function validateListSearch(search: Record<string, unknown>): ListSearch {
  const state = STATES.find((candidate) => candidate === search.state);
  const q = typeof search.q === "string" && search.q.trim() ? search.q : undefined;
  return {
    state,
    q,
    new: search.new === true || search.new === "true" ? true : undefined,
  };
}

/** Whether `text` is in any of `fields`, ignoring case. */
export function matchesQuery(query: string | undefined, ...fields: (string | null | undefined)[]) {
  const needle = query?.trim().toLowerCase();
  if (!needle) return true;
  return fields.some((field) => field?.toLowerCase().includes(needle));
}
