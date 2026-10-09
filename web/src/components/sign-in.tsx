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
    <main className="mx-auto flex min-h-screen max-w-sm flex-col justify-center gap-4 p-6">
      <h1 className="text-xl font-semibold">Sign in to Igloo</h1>
      <form onSubmit={submit} className="flex flex-col gap-3">
        <label htmlFor="token" className="text-sm">
          API token
        </label>
        <Input
          id="token"
          type="password"
          autoComplete="off"
          value={token}
          onChange={(event) => setToken(event.target.value)}
          aria-invalid={error !== null}
          aria-describedby={error ? "token-error" : undefined}
        />
        {error ? (
          <p id="token-error" role="alert" className="text-sm text-destructive">
            {error}
          </p>
        ) : null}
        <Button type="submit" disabled={busy || token.trim() === ""}>
          Sign in
        </Button>
      </form>
    </main>
  );
}
