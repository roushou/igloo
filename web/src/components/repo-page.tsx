import type { ReactNode } from "react";
import type { Repo } from "@/api/client";
import { Empty, ErrorNote, Loading } from "@/components/page";
import { useCurrentRepo } from "@/lib/selected-repo";

/**
 * Renders `children` for the current repository, or says why there is none. `placeholder` stands
 * in for the page while the repositories load, such as the frame of a list.
 */
export function WithRepo({
  children,
  placeholder,
}: {
  children: (repo: Repo) => ReactNode;
  placeholder?: ReactNode;
}) {
  const { repo, pending, error, retry } = useCurrentRepo();
  if (pending) return placeholder ?? <Loading what="repositories" />;
  if (error) return <ErrorNote error={error} what="repositories" onRetry={retry} />;
  if (!repo) return <Empty>No repository is registered. Add one with `igloo repo add`.</Empty>;
  return <>{children(repo)}</>;
}
