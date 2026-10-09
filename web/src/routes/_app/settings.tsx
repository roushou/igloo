import { createFileRoute } from "@tanstack/react-router";
import { Placeholder } from "@/components/placeholder";

export const Route = createFileRoute("/_app/settings")({
  component: () => <Placeholder title="Settings" />,
});
