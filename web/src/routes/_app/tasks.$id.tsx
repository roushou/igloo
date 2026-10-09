import { createFileRoute } from "@tanstack/react-router";
import { Placeholder } from "@/components/placeholder";

export const Route = createFileRoute("/_app/tasks/$id")({
  component: Page,
});

function Page() {
  const { id } = Route.useParams();
  return <Placeholder title="Task" detail={id} />;
}
