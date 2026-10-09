import { createFileRoute } from "@tanstack/react-router";
import { TasksLayout } from "@/components/pages/tasks-page";
import { validateListSearch } from "@/lib/list-search";

export const Route = createFileRoute("/_app/tasks")({
  validateSearch: validateListSearch,
  component: TasksLayout,
});
