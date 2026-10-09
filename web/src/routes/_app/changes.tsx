import { createFileRoute } from "@tanstack/react-router";
import { ChangesLayout } from "@/components/pages/changes-page";
import { validateListSearch } from "@/lib/list-search";

export const Route = createFileRoute("/_app/changes")({
  validateSearch: validateListSearch,
  component: ChangesLayout,
});
