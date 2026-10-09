import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { id, job, run, sandbox, task, worker } from "@/test/fixtures";
import { REPO, renderApp, stubServer } from "@/test/render-app";
import { signIn, TOKEN } from "@/test/server";

afterEach(() => {
  vi.unstubAllGlobals();
});

const repoRoutes = {
  [`GET /v1/repos/${REPO.id}/tasks`]: [],
  [`GET /v1/repos/${REPO.id}/runs`]: { items: [] },
};

describe("System", () => {
  it("shows each worker's connection, allocation, disk and layer cache", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      ...repoRoutes,
      "GET /v1/workers": [
        worker(1),
        worker(2, {
          connection: "lost",
          disconnected_since: "2026-01-01T09:00:00Z",
          schedulability: "draining",
          schedulable: false,
          usage: null,
        }),
      ],
      "GET /v1/sandboxes": { items: [] },
    });
    renderApp("/system");
    const first = await screen.findByRole("article", { name: `Worker ${id("wrk", 1)}` });
    expect(within(first).getByText("Connected")).toBeInTheDocument();
    expect(within(first).getByText("3 CPUs")).toBeInTheDocument();
    expect(within(first).getByText("6.0 GiB")).toBeInTheDocument();
    expect(within(first).getByRole("meter", { name: "Disk" })).toHaveAttribute(
      "aria-valuetext",
      "60.0 GiB used of 100 GiB",
    );
    expect(within(first).getByRole("meter", { name: "Layer cache" })).toHaveAttribute(
      "aria-valuetext",
      "6.0 GiB of 20.0 GiB",
    );
    expect(within(first).getByText("Schedulable")).toBeInTheDocument();

    const second = screen.getByRole("article", { name: `Worker ${id("wrk", 2)}` });
    expect(within(second).getByText("Lost")).toBeInTheDocument();
    expect(within(second).getByText(/Draining/)).toBeInTheDocument();
    expect(within(second).getByText(/Not connected since/)).toBeInTheDocument();
    expect(within(second).getByText("This worker has not reported its usage.")).toBeInTheDocument();
    expect(within(second).queryByRole("meter")).not.toBeInTheDocument();
  });

  it("says when no worker has connected", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      ...repoRoutes,
      "GET /v1/workers": [],
      "GET /v1/sandboxes": { items: [] },
    });
    renderApp("/system");
    expect(await screen.findByText("No worker has connected.")).toBeInTheDocument();
    expect(await screen.findByText("No sandbox is running.")).toBeInTheDocument();
  });

  it("lists sandboxes with the task or run that owns them", async () => {
    signIn();
    const building = run(5, { warm_job: id("job", 55), checks: [] });
    stubServer(TOKEN, [REPO], {
      [`GET /v1/repos/${REPO.id}/tasks`]: [task(1)],
      [`GET /v1/repos/${REPO.id}/runs`]: { items: [building] },
      [`GET /v1/jobs/${id("job", 55)}`]: job(55, { sandbox: id("sbx", 5) }),
      "GET /v1/workers": [worker(1)],
      "GET /v1/sandboxes": {
        items: [
          sandbox(1),
          sandbox(5),
          sandbox(9),
          sandbox(7, { phase: "stopped", desired: "stopped" }),
          sandbox(8, { phase: "failed", failure_reason: "image pull failed" }),
        ],
      },
    });
    renderApp("/system");
    const live = await screen.findByRole("list", { name: "Sandboxes" });
    expect(
      await within(live).findByRole("link", { name: `Task ${"task_0000…0001"}` }),
    ).toHaveAttribute("href", `/tasks/${id("task", 1)}`);
    expect(await within(live).findByRole("link", { name: "Run run_0000…0005" })).toHaveAttribute(
      "href",
      `/runs/${id("run", 5)}`,
    );
    expect(within(live).getAllByText("No task or run in progress")).toHaveLength(2);
    expect(within(live).getByText("image pull failed")).toBeInTheDocument();
    expect(within(live).getAllByText("Running").length).toBe(3);
    const stopped = screen.getByRole("list", { name: "Stopped sandboxes" });
    expect(within(stopped).getByText("Stopped")).toBeInTheDocument();
  });

  it("marks the sandbox named in the address", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      ...repoRoutes,
      "GET /v1/workers": [],
      "GET /v1/sandboxes": { items: [sandbox(1), sandbox(2)] },
    });
    renderApp(`/system?sandbox=${id("sbx", 2)}`);
    const live = await screen.findByRole("list", { name: "Sandboxes" });
    const rows = await within(live).findAllByRole("listitem");
    expect(rows[0]).not.toHaveAttribute("aria-current");
    expect(rows[1]).toHaveAttribute("aria-current", "true");
  });
});

describe("Repository", () => {
  function serveSecrets(names: string[], repo = REPO) {
    const secrets = [...names];
    const server = stubServer(TOKEN, [repo], {
      [`GET /v1/repos/${repo.id}/secrets`]: () => ({ names: secrets }),
      [`PUT /v1/repos/${repo.id}/secrets/NEW_TOKEN`]: (_: Request, body: unknown) => {
        secrets.push("NEW_TOKEN");
        void body;
        return new Response(null, { status: 204 });
      },
      [`DELETE /v1/repos/${repo.id}/secrets/OLD_TOKEN`]: () => {
        secrets.splice(secrets.indexOf("OLD_TOKEN"), 1);
        return new Response(null, { status: 204 });
      },
    });
    return server;
  }

  it("shows the forge location and token secret name", async () => {
    signIn();
    const repo = { ...REPO, token_secret: "FORGE_TOKEN" };
    serveSecrets(["FORGE_TOKEN"], repo);
    renderApp("/settings");
    const forge = await screen.findByRole("region", { name: "Forge" });
    expect(within(forge).getByText(REPO.location)).toBeInTheDocument();
    expect(within(forge).getByText("FORGE_TOKEN")).toBeInTheDocument();
    expect(within(forge).getByText("main")).toBeInTheDocument();
    expect(await screen.findByText("forge token")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("warns when the token secret is not set", async () => {
    signIn();
    serveSecrets([], { ...REPO, token_secret: "FORGE_TOKEN" } as typeof REPO);
    renderApp("/settings");
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The forge token secret FORGE_TOKEN is not set.",
    );
  });

  it("sets a secret through a masked input and never shows its value", async () => {
    signIn();
    const { calls } = serveSecrets(["OLD_TOKEN"]);
    renderApp("/settings");
    await screen.findByText("OLD_TOKEN");
    const form = screen.getByRole("form", { name: "Set a secret" });
    const value = within(form).getByLabelText("Value");
    expect(value).toHaveAttribute("type", "password");
    const submit = within(form).getByRole("button", { name: "Set secret" });
    expect(submit).toBeDisabled();
    expect(submit).toHaveAccessibleDescription("Name the secret");

    await userEvent.type(within(form).getByLabelText("Name"), "new_token");
    expect(submit).toHaveAccessibleDescription(/capital letters/);
    await userEvent.clear(within(form).getByLabelText("Name"));
    await userEvent.type(within(form).getByLabelText("Name"), "NEW_TOKEN");
    expect(submit).toHaveAccessibleDescription("Enter the value");
    await userEvent.type(value, "hunter2-secret");
    expect(submit).toBeEnabled();
    await userEvent.click(submit);

    const put = await waitFor(() => {
      const call = calls.find((c) => c.method === "PUT");
      expect(call).toBeDefined();
      return call;
    });
    expect(put?.path).toBe(`/v1/repos/${REPO.id}/secrets/NEW_TOKEN`);
    expect(put?.body).toBe("hunter2-secret");

    const list = await screen.findByRole("list", { name: "Secrets" });
    expect(await within(list).findByText("NEW_TOKEN")).toBeInTheDocument();
    expect(within(form).getByLabelText("Value")).toHaveValue("");
    expect(within(form).getByLabelText("Name")).toHaveValue("");
    expect(document.body.innerHTML).not.toContain("hunter2-secret");
    expect(screen.queryByDisplayValue("hunter2-secret")).not.toBeInTheDocument();
  });

  it("shows the CLI command of setting a secret", async () => {
    signIn();
    serveSecrets([]);
    renderApp("/settings");
    await screen.findByText("No secret is set.");
    await userEvent.click(screen.getByRole("button", { name: "Terminal commands" }));
    const popover = await screen.findByRole("dialog");
    expect(within(popover).getByText("Set a secret")).toBeInTheDocument();
    expect(
      within(popover).getByText(`printf %s "$VALUE" | igloo secret set ${REPO.id} <NAME>`),
    ).toBeInTheDocument();
  });

  it("deletes a secret after confirmation", async () => {
    signIn();
    const { calls } = serveSecrets(["OLD_TOKEN"]);
    renderApp("/settings");
    await userEvent.click(await screen.findByRole("button", { name: "Delete OLD_TOKEN" }));
    expect(calls.some((c) => c.method === "DELETE")).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "Confirm delete OLD_TOKEN" }));
    expect(await screen.findByText("No secret is set.")).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "Secrets" })).not.toBeInTheDocument();
    expect(calls.find((c) => c.method === "DELETE")?.path).toBe(
      `/v1/repos/${REPO.id}/secrets/OLD_TOKEN`,
    );
    expect(await screen.findByText("Secret deleted")).toBeInTheDocument();
  });

  it("says when a secret cannot be set", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [`GET /v1/repos/${REPO.id}/secrets`]: { names: [] },
      [`PUT /v1/repos/${REPO.id}/secrets/NEW_TOKEN`]: () =>
        Response.json(
          {
            type: "about:blank",
            title: "Unprocessable",
            status: 422,
            code: "secret.invalid",
            detail: "the value is too long",
          },
          { status: 422 },
        ),
    });
    renderApp("/settings");
    await screen.findByText("No secret is set.");
    await userEvent.type(screen.getByLabelText("Name"), "NEW_TOKEN");
    await userEvent.type(screen.getByLabelText("Value"), "x");
    await userEvent.click(screen.getByRole("button", { name: "Set secret" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("the value is too long");
  });
});
