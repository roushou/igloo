import { createFileRoute } from "@tanstack/react-router";
import { ChangesPage } from "@/components/pages/changes-page";

export const Route = createFileRoute("/_app/changes/")({ component: ChangesPage });
