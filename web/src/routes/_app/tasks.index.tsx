import { createFileRoute } from "@tanstack/react-router";

// The list is the layout's: with no task open there is nothing beside it.
export const Route = createFileRoute("/_app/tasks/")({ component: () => null });
