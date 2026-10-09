import { createFileRoute } from "@tanstack/react-router";
import { Placeholder } from "@/components/placeholder";

export const Route = createFileRoute("/_app/changes/")({
  component: () => <Placeholder title="Changes" />,
});
