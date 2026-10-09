import { createFileRoute, redirect } from "@tanstack/react-router";
import { api } from "@/api/client";
import { SignIn } from "@/components/sign-in";

export const Route = createFileRoute("/signin")({
  beforeLoad: () => {
    if (api.tokens.get()) throw redirect({ to: "/" });
  },
  component: SignIn,
});
