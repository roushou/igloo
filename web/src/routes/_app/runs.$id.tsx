import { createFileRoute } from "@tanstack/react-router";
import { Placeholder } from "@/components/placeholder";

export const Route = createFileRoute("/_app/runs/$id")({
  component: Page,
});

function Page() {
  const { id } = Route.useParams();
  return <Placeholder title="Run" detail={id} />;
}
