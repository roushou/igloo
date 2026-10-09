import { createFileRoute } from "@tanstack/react-router";
import { SystemPage } from "@/components/pages/system-page";

export const Route = createFileRoute("/_app/system")({
  validateSearch: (search: Record<string, unknown>) => ({
    sandbox: typeof search.sandbox === "string" ? search.sandbox : undefined,
  }),
  component: Page,
});

function Page() {
  const { sandbox } = Route.useSearch();
  return <SystemPage sandbox={sandbox} />;
}
