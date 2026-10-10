import { createFileRoute } from "@tanstack/react-router";
import { WorkspacesPage } from "@/components/pages/workspaces-page";
import { validateListSearch } from "@/lib/list-search";

export const Route = createFileRoute("/_app/workspaces/")({
  validateSearch: validateListSearch,
  component: WorkspacesPage,
});
