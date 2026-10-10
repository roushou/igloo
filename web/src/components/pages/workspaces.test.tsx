import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { change, id, me, run, workspace } from "@/test/fixtures";
import { REPO, renderApp, stubServer } from "@/test/render-app";
import { signIn, TOKEN } from "@/test/server";

/** The terminal records the sandbox the page opens it in; its own tests cover the rest. */
vi.mock("@/components/terminal", () => ({
  Terminal: ({ sandboxId, mode }: { sandboxId: string; mode?: string }) => (
    <div role="application" aria-label="Terminal" data-sandbox={sandboxId} data-mode={mode} />
  ),
}));

afterEach(() => {
  vi.unstubAllGlobals();
});

const listPath = "GET /v1/workspaces";
const onePath = (n: number) => `GET /v1/workspaces/${id("wsp", n)}`;
const changesPath = `GET /v1/repos/${REPO.id}/changes`;
const changeRunsPath = (n: number) => `GET /v1/changes/${id("chg", n)}/runs`;

describe("Workspaces page", () => {
  it("lists the workspaces with phase, branch and last activity", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [listPath]: [
        workspace(1, { phase: "running" }),
        workspace(2, { phase: "stopped", sandbox: undefined }),
        workspace(3, {
          phase: "running",
          setup: { state: "failed", job: id("job", 9), reason: "exit 1" },
        }),
      ],
    });
    renderApp("/workspaces");
    const list = await screen.findByRole("list", { name: "Workspaces" });
    const rows = within(list).getAllByRole("listitem");
    expect(rows).toHaveLength(3);
    expect(within(rows[0] as HTMLElement).getByText("feature-1")).toBeInTheDocument();
    expect(within(rows[0] as HTMLElement).getByText("Running")).toBeInTheDocument();
    expect(within(rows[1] as HTMLElement).getByText("Stopped")).toBeInTheDocument();
    expect(within(rows[2] as HTMLElement).getByText("Setup failed")).toBeInTheDocument();
  });

  it("stops a running workspace and starts a stopped one", async () => {
    signIn();
    const { calls } = stubServer(TOKEN, [REPO], {
      [listPath]: [workspace(1), workspace(2, { phase: "stopped", sandbox: undefined })],
      [`POST /v1/workspaces/${id("wsp", 1)}/stop`]: workspace(1, { phase: "stopping" }),
      [`POST /v1/workspaces/${id("wsp", 2)}/start`]: workspace(2, { phase: "starting" }),
    });
    renderApp("/workspaces");
    await userEvent.click(await screen.findByRole("button", { name: "Stop" }));
    await userEvent.click(screen.getByRole("button", { name: "Start" }));
    await waitFor(() =>
      expect(calls.map((call) => `${call.method} ${call.path}`)).toEqual(
        expect.arrayContaining([
          `POST /v1/workspaces/${id("wsp", 1)}/stop`,
          `POST /v1/workspaces/${id("wsp", 2)}/start`,
        ]),
      ),
    );
  });

  it("deletes a workspace after asking", async () => {
    signIn();
    const { calls } = stubServer(TOKEN, [REPO], {
      [listPath]: [workspace(1)],
      [`DELETE /v1/workspaces/${id("wsp", 1)}`]: new Response(null, { status: 204 }),
    });
    renderApp("/workspaces");
    await userEvent.click(await screen.findByRole("button", { name: "Delete" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(calls.some((call) => call.method === "DELETE")).toBe(false);
    await userEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(calls.some((call) => call.method === "DELETE")).toBe(true));
  });

  it("creates a workspace from a branch", async () => {
    signIn();
    const { calls } = stubServer(TOKEN, [REPO], {
      [listPath]: [],
      [`POST /v1/repos/${REPO.id}/workspaces`]: workspace(4, {
        phase: "starting",
        sandbox: undefined,
      }),
      [onePath(4)]: workspace(4, { phase: "starting", sandbox: undefined }),
      [changesPath]: [],
    });
    renderApp("/workspaces");
    expect(await screen.findByText("No workspace yet")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Create workspace" }));
    await userEvent.type(await screen.findByLabelText("Branch"), "feature-4");
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() =>
      expect(calls.find((call) => call.method === "POST")?.body).toEqual({ branch: "feature-4" }),
    );
    expect(await screen.findByText(/The sandbox is starting/)).toBeInTheDocument();
  });
});

describe("Workspace page", () => {
  const open = (n = 1) => renderApp(`/workspaces/${id("wsp", n)}`);

  it("opens the terminal of a running workspace", async () => {
    signIn();
    stubServer(TOKEN, [REPO], { [onePath(1)]: workspace(1), [changesPath]: [] });
    open();
    expect(await screen.findByRole("application", { name: "Terminal" })).toHaveAttribute(
      "data-sandbox",
      id("sbx", 51),
    );
    expect(screen.getByText("Running")).toBeInTheDocument();
  });

  it("waits for a starting workspace", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [onePath(1)]: workspace(1, { phase: "starting", sandbox: undefined }),
      [changesPath]: [],
    });
    open();
    expect(await screen.findByText(/The sandbox is starting/)).toBeInTheDocument();
    expect(screen.getByText("Starting")).toBeInTheDocument();
    expect(screen.queryByRole("application", { name: "Terminal" })).not.toBeInTheDocument();
  });

  it("says a stopping workspace is sealing its changes", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [onePath(1)]: workspace(1, { phase: "stopping" }),
      [changesPath]: [],
    });
    open();
    expect(await screen.findByText(/Sealing your changes/)).toBeInTheDocument();
    expect(screen.queryByRole("application", { name: "Terminal" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Stop" })).toBeDisabled();
  });

  it("starts a stopped workspace", async () => {
    signIn();
    let current = workspace(1, { phase: "stopped", sandbox: undefined });
    const { calls } = stubServer(TOKEN, [REPO], {
      [onePath(1)]: () => current,
      [`POST /v1/workspaces/${id("wsp", 1)}/start`]: () => {
        current = workspace(1, { phase: "starting", sandbox: undefined });
        return current;
      },
      [changesPath]: [],
    });
    open();
    expect(await screen.findByText(/Stopped. Start it to resume/)).toBeInTheDocument();
    expect(screen.queryByRole("application", { name: "Terminal" })).not.toBeInTheDocument();
    await userEvent.click(screen.getAllByRole("button", { name: "Start" })[0] as HTMLElement);
    await waitFor(() => expect(calls.some((call) => call.path.endsWith("/start"))).toBe(true));
    expect(await screen.findByText(/The sandbox is starting/)).toBeInTheDocument();
  });

  it("shows a failed setup with its reason and the log, and keeps the terminal", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [onePath(1)]: workspace(1, {
        setup: { state: "failed", job: id("job", 9), reason: "dotfiles install exited 1" },
      }),
      [changesPath]: [],
    });
    open();
    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText(/dotfiles install exited 1/)).toBeInTheDocument();
    expect(within(alert).getByRole("link", { name: "Open the setup log" })).toHaveAttribute(
      "href",
      `/jobs/${id("job", 9)}`,
    );
    expect(screen.getByRole("application", { name: "Terminal" })).toBeInTheDocument();
  });

  it("says a deleted workspace is gone", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [onePath(1)]: Response.json(
        { title: "Not found", status: 404, code: "workspace.not_found", type: "about:blank" },
        { status: 404 },
      ),
    });
    open();
    expect(await screen.findByText(/This workspace was deleted/)).toBeInTheDocument();
    expect(screen.queryByRole("application", { name: "Terminal" })).not.toBeInTheDocument();
  });

  it("shows the branch's change, its checks and recent pushes beside the terminal", async () => {
    signIn();
    const revisions = [1, 2].map((number) => ({
      number,
      base: "a".repeat(40),
      head: `${number}`.repeat(40),
      created_at: "2026-01-01T10:00:00Z",
    }));
    stubServer(TOKEN, [REPO], {
      [onePath(1)]: workspace(1),
      [changesPath]: [
        change(3, { title: "Other branch", source_branch: "other" }),
        change(5, { title: "Feature work", source_branch: "feature-1", revisions }),
      ],
      [changeRunsPath(5)]: [run(5, { change: id("chg", 5), revision: 2 })],
    });
    open();
    const panel = await screen.findByRole("complementary", { name: "Branch" });
    expect(await within(panel).findByText("Feature work")).toBeInTheDocument();
    expect(within(panel).queryByText("Other branch")).not.toBeInTheDocument();
    const pushes = within(panel).getByRole("list", { name: "Recent pushes" });
    expect(
      within(pushes)
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual([expect.stringContaining("Revision 2"), expect.stringContaining("Revision 1")]);
    expect(await within(panel).findByRole("img")).toBeInTheDocument();
  });

  it("says when no change proposes the branch", async () => {
    signIn();
    stubServer(TOKEN, [REPO], { [onePath(1)]: workspace(1), [changesPath]: [] });
    open();
    expect(await screen.findByText(/No change proposes feature-1/)).toBeInTheDocument();
    expect(screen.getByText("Nothing pushed yet.")).toBeInTheDocument();
  });
});

describe("Now", () => {
  it("shows running workspaces beside running tasks", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [`GET /v1/repos/${REPO.id}/tasks`]: [],
      [changesPath]: [],
      [`GET /v1/repos/${REPO.id}/runs`]: { items: [] },
      [listPath]: [workspace(1), workspace(2, { phase: "stopped", sandbox: undefined })],
      "GET /v1/me": me(),
    });
    renderApp("/");
    const running = await screen.findByRole("region", { name: "Running" });
    expect(await within(running).findByText("Workspace on feature-1")).toBeInTheDocument();
    expect(within(running).queryByText("Workspace on feature-2")).not.toBeInTheDocument();
  });
});
