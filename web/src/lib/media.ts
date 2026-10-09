import { useSyncExternalStore } from "react";

/** The width from which the console shows a list beside its detail, and the sidebar in place. */
export const WIDE_QUERY = "(min-width: 1024px)";

/** Whether `query` matches now, following changes. False where there is no browser. */
export function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (notify) => {
      if (typeof window.matchMedia !== "function") return () => {};
      const list = window.matchMedia(query);
      list.addEventListener("change", notify);
      return () => list.removeEventListener("change", notify);
    },
    () => typeof window.matchMedia === "function" && window.matchMedia(query).matches,
    () => false,
  );
}

/** Whether the viewport is wide enough for split views and the sidebar. */
export function useIsWide(): boolean {
  return useMediaQuery(WIDE_QUERY);
}
