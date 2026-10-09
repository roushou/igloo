import { createFileRoute, redirect } from "@tanstack/react-router";
import { api } from "@/api/client";
import { AppShell } from "@/components/app-shell";

export const Route = createFileRoute("/_app")({
  beforeLoad: () => {
    if (!api.tokens.get()) throw redirect({ to: "/signin" });
  },
  component: AppShell,
});
