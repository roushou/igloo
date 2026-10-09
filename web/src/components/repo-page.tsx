import type { ReactNode } from "react";
import type { Repo } from "@/api/client";
import { Empty, ErrorNote, Loading } from "@/components/page";
import { useCurrentRepo } from "@/lib/selected-repo";

/** Renders `children` for the current repository, or says why there is none. */
export function WithRepo({ children }: { children: (repo: Repo) => ReactNode }) {
  const { repo, pending, error } = useCurrentRepo();
  if (pending) return <Loading what="repositories" />;
  if (error) return <ErrorNote error={error} what="repositories" />;
  if (!repo) return <Empty>No repository is registered. Add one with `igloo repo add`.</Empty>;
  return <>{children(repo)}</>;
}
