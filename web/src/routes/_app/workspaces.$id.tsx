import { createFileRoute } from "@tanstack/react-router";
import { WorkspacePage } from "@/components/pages/workspace-page";

export const Route = createFileRoute("/_app/workspaces/$id")({ component: Page });

function Page() {
  const { id } = Route.useParams();
  return <WorkspacePage id={id} />;
}
