import { createFileRoute } from "@tanstack/react-router";
import { Placeholder } from "@/components/placeholder";

export const Route = createFileRoute("/_app/")({
  component: () => <Placeholder title="Now" />,
});
