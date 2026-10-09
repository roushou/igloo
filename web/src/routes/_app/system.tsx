import { createFileRoute } from "@tanstack/react-router";
import { Placeholder } from "@/components/placeholder";

export const Route = createFileRoute("/_app/system")({
  validateSearch: (search: Record<string, unknown>) => ({
    sandbox: typeof search.sandbox === "string" ? search.sandbox : undefined,
  }),
  component: Page,
});

function Page() {
  const { sandbox } = Route.useSearch();
  return <Placeholder title="System" detail={sandbox} />;
}
