import { createFileRoute } from "@tanstack/react-router";
import { NowPage } from "@/components/pages/now-page";

export const Route = createFileRoute("/_app/")({ component: NowPage });
