import { createFileRoute } from "@tanstack/react-router";
import { ChangePage } from "@/components/pages/change-page";

export const Route = createFileRoute("/_app/changes/$id")({ component: Page });

function Page() {
  const { id } = Route.useParams();
  return <ChangePage id={id} />;
}
