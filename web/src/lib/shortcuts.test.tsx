import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { SHORTCUTS } from "@/lib/shortcuts";
import { id, task } from "@/test/fixtures";
import { REPO, renderApp, stubServer } from "@/test/render-app";
import { signIn, TOKEN } from "@/test/server";

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

const tasksPath = `GET /v1/repos/${REPO.id}/tasks`;

async function open(path: string, routes: Record<string, unknown> = {}) {
  signIn();
  stubServer(TOKEN, [REPO], { [tasksPath]: [], ...routes });
  const app = renderApp(path);
  await screen.findByRole("navigation", { name: "Main" });
  return app;
}

describe("go to", () => {
  it.each([
    ["n", "/tasks", "/"],
    ["t", "/", "/tasks"],
    ["c", "/", "/changes"],
    ["s", "/", "/system"],
    ["r", "/", "/settings"],
  ])("g then %s goes from %s to %s", async (key, from, to) => {
    const { router } = await open(from);
    await userEvent.keyboard(`g${key}`);
    await waitFor(() => expect(router.state.location.pathname).toBe(to));
  });

  it("does nothing for g followed by a key that is not a page", async () => {
    const { router } = await open("/");
    await userEvent.keyboard("gxt");
    expect(router.state.location.pathname).toBe("/");
  });

  it("forgets g when the second key comes too late", async () => {
    const { router } = await open("/");
    await userEvent.keyboard("g");
    await new Promise((resolve) => setTimeout(resolve, 1_300));
    await userEvent.keyboard("t");
    expect(router.state.location.pathname).toBe("/");
  });

  it("leaves keys typed into a field alone", async () => {
    const { router } = await open("/tasks", {
      [tasksPath]: [task(1, { goal: "Alpha", phase: "awaiting_review" })],
    });
    const filter = await screen.findByRole("textbox", { name: "Filter tasks" });
    await userEvent.type(filter, "gcj?");
    expect(filter).toHaveValue("gcj?");
    expect(router.state.location.pathname).toBe("/tasks");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});

describe("?", () => {
  it("lists every shortcut and closes with Escape", async () => {
    await open("/");
    await userEvent.keyboard("?");
    const dialog = await screen.findByRole("dialog", { name: "Keyboard shortcuts" });
    for (const group of SHORTCUTS) {
      const section = within(dialog).getByRole("region", { name: group.group });
      for (const item of group.items) {
        expect(within(section).getByText(item.label)).toBeInTheDocument();
      }
    }
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("opens from the sidebar too", async () => {
    await open("/");
    await userEvent.click(screen.getByRole("button", { name: "Keyboard shortcuts" }));
    expect(await screen.findByRole("dialog", { name: "Keyboard shortcuts" })).toBeInTheDocument();
  });
});

describe("rows", () => {
  const routes = {
    [tasksPath]: [
      task(1, { goal: "Alpha", phase: "awaiting_review" }),
      task(2, { goal: "Beta", phase: "working" }),
      task(3, { goal: "Gamma", phase: "working" }),
    ],
  };

  it("moves through the rows with j and k and opens one with Enter", async () => {
    const { router } = await open("/tasks", routes);
    const alpha = await screen.findByRole("link", { name: "Alpha" });
    const beta = screen.getByRole("link", { name: "Beta" });
    const gamma = screen.getByRole("link", { name: "Gamma" });

    await userEvent.keyboard("j");
    expect(alpha).toHaveFocus();
    await userEvent.keyboard("j");
    expect(beta).toHaveFocus();
    await userEvent.keyboard("jj");
    expect(alpha).toHaveFocus();
    await userEvent.keyboard("k");
    expect(gamma).toHaveFocus();
    await userEvent.keyboard("k");
    expect(beta).toHaveFocus();

    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(router.state.location.pathname).toBe(`/tasks/${id("task", 2)}`));
  });

  it("starts from the last row with k", async () => {
    await open("/tasks", routes);
    const gamma = await screen.findByRole("link", { name: "Gamma" });
    await userEvent.keyboard("k");
    expect(gamma).toHaveFocus();
  });
});

describe("c and [", () => {
  it("c opens the Create task dialog on the Tasks page", async () => {
    const { router } = await open("/");
    await userEvent.keyboard("c");
    expect(await screen.findByRole("dialog", { name: "Create task" })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/tasks");
    expect(router.state.location.search).toMatchObject({ new: true });
  });

  it("[ collapses and expands the sidebar, and remembers it", async () => {
    await open("/");
    expect(screen.getByRole("button", { name: "Collapse the sidebar" })).toBeInTheDocument();
    await userEvent.keyboard("[[");
    expect(await screen.findByRole("button", { name: "Expand the sidebar" })).toBeInTheDocument();
    expect(localStorage.getItem("igloo.sidebar")).toBe("collapsed");
    await userEvent.keyboard("[[");
    expect(await screen.findByRole("button", { name: "Collapse the sidebar" })).toBeInTheDocument();
    expect(localStorage.getItem("igloo.sidebar")).toBe("expanded");
  });
});

describe("filters in the URL", () => {
  const routes = {
    [tasksPath]: [
      task(1, { goal: "Alpha", phase: "awaiting_review" }),
      task(2, { goal: "Beta", phase: "working" }),
    ],
  };

  it("keeps the state and the text in the search parameters", async () => {
    const { router } = await open("/tasks", routes);
    await userEvent.click(await screen.findByRole("button", { name: /^Running/ }));
    await waitFor(() => expect(router.state.location.search).toMatchObject({ state: "running" }));
    expect(screen.queryByRole("link", { name: "Alpha" })).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Beta" })).toBeInTheDocument();

    await userEvent.type(screen.getByRole("textbox", { name: "Filter tasks" }), "bet");
    await waitFor(() =>
      expect(router.state.location.search).toMatchObject({ state: "running", q: "bet" }),
    );

    await userEvent.click(screen.getByRole("button", { name: /^Running/ }));
    await waitFor(() => expect(router.state.location.search).not.toHaveProperty("state"));
  });

  it("opens a shared link with its filters applied", async () => {
    await open("/tasks?state=running&q=bet", routes);
    expect(await screen.findByRole("link", { name: "Beta" })).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Alpha" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Running/ })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("textbox", { name: "Filter tasks" })).toHaveValue("bet");
  });

  it("says when nothing matches", async () => {
    await open("/tasks?q=zzz", routes);
    expect(await screen.findByText("No task matches the filter.")).toBeInTheDocument();
  });
});
