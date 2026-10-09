import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { api } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

/** Asks for the bearer token, checks it against the server and keeps it. */
export function SignIn() {
  const navigate = useNavigate();
  const [token, setToken] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    const result = await api.check(token.trim());
    setBusy(false);
    if (result === "accepted") {
      api.tokens.set(token.trim());
      await navigate({ to: "/" });
    } else {
      setError(
        result === "rejected"
          ? "The server rejected this token."
          : "The server could not be reached.",
      );
    }
  };

  return (
    <main className="relative flex min-h-dvh items-center justify-center overflow-hidden bg-background p-6">
      <div
        aria-hidden
        className="pointer-events-none absolute inset-x-0 top-0 h-80 bg-[radial-gradient(60%_100%_at_50%_0%,color-mix(in_oklab,var(--muted-foreground)_12%,transparent),transparent)]"
      />
      <div className="relative flex w-full max-w-sm flex-col gap-6 rounded-xl border bg-card p-6 shadow-float">
        <div className="flex flex-col gap-3">
          <span
            aria-hidden
            className="flex size-9 items-center justify-center rounded-lg bg-foreground font-display text-lg font-semibold text-background"
          >
            I
          </span>
          <div className="flex flex-col gap-1">
            <h1 className="text-xl font-semibold tracking-tight">Sign in to Igloo</h1>
            <p className="text-base text-muted-foreground">
              Paste the API token of this server. It stays in this browser.
            </p>
          </div>
        </div>
        <form onSubmit={submit} className="flex flex-col gap-3">
          <label htmlFor="token" className="text-base font-medium">
            API token
          </label>
          <Input
            id="token"
            type="password"
            autoComplete="off"
            autoFocus
            className="h-9 font-mono"
            value={token}
            onChange={(event) => setToken(event.target.value)}
            aria-invalid={error !== null}
            aria-describedby={error ? "token-error" : undefined}
          />
          {error ? (
            <p id="token-error" role="alert" className="text-base text-destructive">
              {error}
            </p>
          ) : null}
          <Button type="submit" size="lg" disabled={busy || token.trim() === ""}>
            {busy ? "Checking…" : "Sign in"}
          </Button>
        </form>
      </div>
    </main>
  );
}
