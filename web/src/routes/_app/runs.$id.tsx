import { createFileRoute } from "@tanstack/react-router";
import { RunPage } from "@/components/pages/run-page";

export const Route = createFileRoute("/_app/runs/$id")({ component: Page });

function Page() {
  const { id } = Route.useParams();
  return <RunPage id={id} />;
}
