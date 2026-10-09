import { createFileRoute } from "@tanstack/react-router";
import { JobPage } from "@/components/pages/job-page";

export const Route = createFileRoute("/_app/jobs/$id")({ component: Page });

function Page() {
  const { id } = Route.useParams();
  return <JobPage id={id} />;
}
