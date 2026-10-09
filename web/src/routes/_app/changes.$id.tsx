import { createFileRoute } from "@tanstack/react-router";
import { Placeholder } from "@/components/placeholder";

export const Route = createFileRoute("/_app/changes/$id")({
  component: Page,
});

function Page() {
  const { id } = Route.useParams();
  return <Placeholder title="Change" detail={id} />;
}
