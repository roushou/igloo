import { createFileRoute } from "@tanstack/react-router";

// The list is the layout's: with no change open there is nothing beside it.
export const Route = createFileRoute("/_app/changes/")({ component: () => null });
