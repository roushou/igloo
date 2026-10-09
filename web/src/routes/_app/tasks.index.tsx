import { createFileRoute } from "@tanstack/react-router";
import { Placeholder } from "@/components/placeholder";

export const Route = createFileRoute("/_app/tasks/")({
  component: () => <Placeholder title="Tasks" />,
});
