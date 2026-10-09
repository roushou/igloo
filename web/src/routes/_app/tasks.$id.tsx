import { createFileRoute } from "@tanstack/react-router";
import { TaskPage } from "@/components/pages/task-page";

export const Route = createFileRoute("/_app/tasks/$id")({ component: Page });

function Page() {
  const { id } = Route.useParams();
  return <TaskPage id={id} />;
}
