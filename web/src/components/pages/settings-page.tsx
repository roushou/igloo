import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound, Trash2, TriangleAlert } from "lucide-react";
import { useState } from "react";
import { api, type Repo } from "@/api/client";
import { ActionButton, CliMenu } from "@/components/action-button";
import { EmptyState } from "@/components/empty-state";
import { Await, Page, Section } from "@/components/page";
import { WithRepo } from "@/components/repo-page";
import { ShortId } from "@/components/short-id";
import { Input } from "@/components/ui/input";
import { useToast } from "@/components/ui/toast";
import { queries, queryKeys } from "@/lib/queries";

/** Repository: where it lives and the secrets its jobs may name. Values are never shown. */
export function SettingsPage() {
  return <WithRepo>{(repo) => <Settings repo={repo} />}</WithRepo>;
}

function Settings({ repo }: { repo: Repo }) {
  const secrets = useQuery(queries.secrets(repo.id));
  return (
    <Page
      crumbs={[{ label: "Repository" }]}
      title="Repository"
      actions={
        <CliMenu
          commands={[
            {
              label: "Set a secret",
              command: `printf %s "$VALUE" | igloo secret set ${repo.id} <NAME>`,
            },
            { label: "Delete a secret", command: `igloo secret delete ${repo.id} <NAME>` },
          ]}
        />
      }
    >
      <div className="flex flex-col gap-8">
        <Section title="Forge">
          <dl className="divide-y divide-border border-y text-base">
            <Row label="Location">
              <span className="font-mono text-sm break-all">{repo.location}</span>
            </Row>
            <Row label="Default branch">
              <span className="font-mono text-sm">{repo.default_branch}</span>
            </Row>
            <Row label="Token secret">
              <span className="font-mono text-sm">{repo.token_secret ?? "none"}</span>
            </Row>
            <Row label="Repository">
              <ShortId id={repo.id} className="-ml-1" />
            </Row>
          </dl>
        </Section>
        <Await query={secrets} what="secrets">
          {(names) => (
            <Section title="Secrets" count={names.length}>
              {repo.token_secret && !names.includes(repo.token_secret) ? (
                <p
                  role="alert"
                  className="flex items-center gap-2 rounded-lg border border-errored/30 bg-errored-soft px-4 py-3 text-base text-errored"
                >
                  <TriangleAlert className="size-4 shrink-0" />
                  The forge token secret {repo.token_secret} is not set.
                </p>
              ) : null}
              {names.length === 0 ? (
                <EmptyState
                  icon={KeyRound}
                  title="No secret is set."
                  command={`printf %s "$VALUE" | igloo secret set ${repo.id} GITHUB_TOKEN`}
                >
                  Secrets are values a job may name, such as a forge token. They are sent once and
                  never shown again.
                </EmptyState>
              ) : (
                <ul aria-label="Secrets" className="divide-y divide-border border-y">
                  {names.map((name) => (
                    <SecretRow key={name} repo={repo} name={name} />
                  ))}
                </ul>
              )}
              <SetSecret repo={repo} />
            </Section>
          )}
        </Await>
      </div>
    </Page>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[8rem_minmax(0,1fr)] items-center gap-x-4 px-4 py-2.5 @2xl:grid-cols-[12rem_minmax(0,1fr)]">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0">{children}</dd>
    </div>
  );
}

function SecretRow({ repo, name }: { repo: Repo; name: string }) {
  const queryClient = useQueryClient();
  const toast = useToast();
  const remove = useMutation({
    mutationFn: () => api.deleteSecret(repo.id, name),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.secrets(repo.id) });
      toast.show({ title: "Secret deleted", description: name });
    },
  });
  const [confirming, setConfirming] = useState(false);
  return (
    <li className="flex items-center gap-3 px-4 py-2">
      <KeyRound className="size-4 shrink-0 text-muted-foreground" />
      <span className="font-mono text-base">{name}</span>
      {name === repo.token_secret ? (
        <span className="rounded-full bg-muted px-2 py-0.5 text-sm text-muted-foreground">
          forge token
        </span>
      ) : null}
      {remove.isError ? (
        <span role="alert" className="text-sm text-failed">
          {remove.error.message}
        </span>
      ) : null}
      <div className="ml-auto flex items-center gap-1">
        {confirming ? (
          <>
            <ActionButton
              label="Cancel"
              variant="ghost"
              size="sm"
              onRun={() => setConfirming(false)}
            />
            <ActionButton
              label={`Confirm delete ${name}`}
              text="Delete it"
              variant="danger"
              size="sm"
              pending={remove.isPending}
              onRun={() => remove.mutate()}
            />
          </>
        ) : (
          <ActionButton
            label={`Delete ${name}`}
            text="Delete"
            icon={<Trash2 />}
            variant="ghost"
            size="sm"
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
  const toast = useToast();
  const set = useMutation({
    mutationFn: () => api.setSecret(repo.id, name.trim(), value),
    onSuccess: async () => {
      toast.show({ title: "Secret set", description: name.trim() });
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
      className="mt-4 flex max-w-2xl flex-col gap-3"
      onSubmit={(event) => {
        event.preventDefault();
        if (!unmet) set.mutate();
      }}
    >
      <p className="text-base font-medium">Set a secret</p>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="flex flex-col gap-1.5 text-base">
          <label htmlFor="secret-name" className="text-muted-foreground">
            Name
          </label>
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
        <div className="flex flex-col gap-1.5 text-base">
          <label htmlFor="secret-value" className="text-muted-foreground">
            Value
          </label>
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
      <p className="text-sm text-muted-foreground">
        The value is sent once and is never shown or returned again. Setting an existing name
        replaces its value.
      </p>
      {set.isError ? (
        <p role="alert" className="text-base text-failed">
          {set.error.message}
        </p>
      ) : null}
      <div className="flex items-start gap-3">
        <ActionButton
          label="Set secret"
          type="submit"
          variant="default"
          unmet={unmet}
          pending={set.isPending}
          onRun={() => {}}
        />
      </div>
    </form>
  );
}
