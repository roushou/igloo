import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api, type Repo } from "@/api/client";
import { ActionButton } from "@/components/action-button";
import { Await, Empty, Page, Rows, Section } from "@/components/page";
import { WithRepo } from "@/components/repo-page";
import { ShortId } from "@/components/short-id";
import { Input } from "@/components/ui/input";
import { queries, queryKeys } from "@/lib/queries";

/** Repository: where it lives and the secrets its jobs may name. Values are never shown. */
export function SettingsPage() {
  return (
    <Page title="Repository">
      <WithRepo>{(repo) => <Settings repo={repo} />}</WithRepo>
    </Page>
  );
}

function Settings({ repo }: { repo: Repo }) {
  const secrets = useQuery(queries.secrets(repo.id));
  return (
    <div className="flex flex-col gap-8">
      <Section title="Forge">
        <dl className="grid max-w-2xl grid-cols-[10rem_1fr] gap-y-2 rounded-lg border bg-card p-4 text-sm">
          <dt className="text-muted-foreground">Location</dt>
          <dd className="font-mono break-all">{repo.location}</dd>
          <dt className="text-muted-foreground">Default branch</dt>
          <dd className="font-mono">{repo.default_branch}</dd>
          <dt className="text-muted-foreground">Token secret</dt>
          <dd className="font-mono">{repo.token_secret ?? "none"}</dd>
          <dt className="text-muted-foreground">Repository</dt>
          <dd>
            <ShortId id={repo.id} />
          </dd>
        </dl>
      </Section>
      <Await query={secrets} what="secrets">
        {(names) => (
          <Section title="Secrets" count={names.length}>
            {repo.token_secret && !names.includes(repo.token_secret) ? (
              <p role="alert" className="text-sm text-errored">
                The forge token secret {repo.token_secret} is not set.
              </p>
            ) : null}
            {names.length === 0 ? (
              <Empty>No secret is set.</Empty>
            ) : (
              <Rows label="Secrets">
                {names.map((name) => (
                  <SecretRow key={name} repo={repo} name={name} />
                ))}
              </Rows>
            )}
            <SetSecret repo={repo} />
          </Section>
        )}
      </Await>
    </div>
  );
}

function SecretRow({ repo, name }: { repo: Repo; name: string }) {
  const queryClient = useQueryClient();
  const remove = useMutation({
    mutationFn: () => api.deleteSecret(repo.id, name),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.secrets(repo.id) }),
  });
  const [confirming, setConfirming] = useState(false);
  return (
    <li className="flex items-center gap-3 px-4 py-2.5">
      <span className="font-mono text-sm">{name}</span>
      {name === repo.token_secret ? (
        <span className="text-xs text-muted-foreground">forge token</span>
      ) : null}
      {remove.isError ? (
        <span role="alert" className="text-xs text-failed">
          {remove.error.message}
        </span>
      ) : null}
      <div className="ml-auto">
        {confirming ? (
          <ActionButton
            label={`Confirm delete ${name}`}
            cli={`igloo secret delete ${repo.id} ${name}`}
            pending={remove.isPending}
            onRun={() => remove.mutate()}
          />
        ) : (
          <ActionButton
            label={`Delete ${name}`}
            cli={`igloo secret delete ${repo.id} ${name}`}
            onRun={() => setConfirming(true)}
          />
        )}
      </div>
    </li>
  );
}

const NAME = /^[A-Z_][A-Z0-9_]*$/;

function SetSecret({ repo }: { repo: Repo }) {
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const queryClient = useQueryClient();
  const set = useMutation({
    mutationFn: () => api.setSecret(repo.id, name.trim(), value),
    onSuccess: async () => {
      setName("");
      setValue("");
      await queryClient.invalidateQueries({ queryKey: queryKeys.secrets(repo.id) });
    },
  });
  const trimmed = name.trim();
  const unmet = !trimmed
    ? "Name the secret"
    : !NAME.test(trimmed)
      ? "A name has capital letters, digits and underscores, and does not start with a digit"
      : !value
        ? "Enter the value"
        : null;
  return (
    <form
      aria-label="Set a secret"
      className="flex max-w-2xl flex-col gap-3 rounded-lg border bg-card p-4"
      onSubmit={(event) => {
        event.preventDefault();
        if (!unmet) set.mutate();
      }}
    >
      <p className="text-sm font-medium">Set a secret</p>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="flex flex-col gap-1 text-sm">
          <label htmlFor="secret-name">Name</label>
          <Input
            id="secret-name"
            autoComplete="off"
            spellCheck={false}
            className="font-mono"
            placeholder="GITHUB_TOKEN"
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
        </div>
        <div className="flex flex-col gap-1 text-sm">
          <label htmlFor="secret-value">Value</label>
          <Input
            id="secret-value"
            type="password"
            autoComplete="new-password"
            spellCheck={false}
            className="font-mono"
            value={value}
            onChange={(event) => setValue(event.target.value)}
          />
        </div>
      </div>
      <p className="text-xs text-muted-foreground">
        The value is sent once and is never shown or returned again. Setting an existing name
        replaces its value.
      </p>
      {set.isError ? (
        <p role="alert" className="text-sm text-failed">
          {set.error.message}
        </p>
      ) : null}
      <div className="flex items-start gap-3">
        <ActionButton
          label="Set secret"
          type="submit"
          variant="default"
          cli={`printf %s "$VALUE" | igloo secret set ${repo.id} ${trimmed || "<NAME>"}`}
          unmet={unmet}
          pending={set.isPending}
          onRun={() => {}}
        />
      </div>
    </form>
  );
}
