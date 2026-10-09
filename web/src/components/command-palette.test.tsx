import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { change, diff, file, id, readiness, task, transcript } from "@/test/fixtures";
import { REPO, renderApp, stubServer } from "@/test/render-app";
import { signIn, TOKEN } from "@/test/server";

// The diff library draws into shadow DOM, which the test DOM cannot show.
vi.mock("@pierre/diffs/react", () => ({ PatchDiff: () => null }));

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

const tasksPath = `GET /v1/repos/${REPO.id}/tasks`;
const changePath = `/v1/changes/${id("chg", 1)}`;
const taskPath = `/v1/tasks/${id("task", 1)}`;

async function palette(path: string, routes: Record<string, unknown> = {}, repos = [REPO]) {
  signIn();
  const server = stubServer(TOKEN, repos, { [tasksPath]: [], ...routes });
  const app = renderApp(path);
  await screen.findByRole("navigation", { name: "Main" });
  await userEvent.keyboard("{Control>}k{/Control}");
  const input = await screen.findByRole("combobox");
  return { ...app, ...server, input };
}

const option = (name: RegExp | string) => screen.findByRole("option", { name });

describe("the command palette", () => {
  it("opens from the sidebar search and closes with Escape", async () => {
    signIn();
    stubServer(TOKEN, [REPO], { [tasksPath]: [] });
    renderApp("/");
    await userEvent.click(await screen.findByRole("button", { name: "Search or run a command" }));
    expect(await screen.findByRole("dialog", { name: "Command palette" })).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("creates a task from the palette", async () => {
    const { router, input } = await palette("/");
    await userEvent.type(input, "create task");
    await userEvent.keyboard("{Enter}");
    expect(await screen.findByRole("dialog", { name: "Create task" })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/tasks");
    expect(screen.queryByRole("dialog", { name: "Command palette" })).not.toBeInTheDocument();
  });

  it.each([
    ["go to changes", "/changes"],
    ["go to system", "/system"],
    ["go to repository", "/settings"],
    ["go to tasks", "/tasks"],
  ])("goes to a page: %s", async (text, path) => {
    const { router, input } = await palette("/");
    await userEvent.type(input, text);
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(router.state.location.pathname).toBe(path));
  });

  it("switches repository", async () => {
    const other = { ...REPO, id: "repo_01j9z3k4m5n6p7q8r9s0t1v2w4", location: "https://x/other" };
    const { input } = await palette("/", {}, [REPO, other]);
    await userEvent.type(input, "switch other");
    await userEvent.click(await option(/Switch to x\/other/));
    await waitFor(() => expect(localStorage.getItem("igloo.repo")).toBe(other.id));
  });

  it("signs out", async () => {
    const { router, input } = await palette("/");
    await userEvent.type(input, "sign out");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(router.state.location.pathname).toBe("/signin"));
  });

  it("shows the shortcut beside a command", async () => {
    await palette("/");
    const create = await option(/Create task/);
    expect(within(create).getByText("C")).toBeInTheDocument();
    const toTasks = await option(/Go to Tasks/);
    expect(within(toTasks).getByText("G")).toBeInTheDocument();
    expect(within(toTasks).getByText("T")).toBeInTheDocument();
  });
});

describe("the commands of the page on screen", () => {
  const serve = (body = change(1), extra: Record<string, unknown> = {}) => ({
    [`GET ${changePath}`]: body,
    [`GET ${changePath}/diff`]: diff([file("src/lib.rs")]),
    [`GET ${changePath}/runs`]: [],
    ...extra,
  });

  it("merges the open change after a confirmation", async () => {
    const merged = change(1, { phase: "merged", merged_commit: "c".repeat(40) });
    const { calls, input } = await palette(
      `/changes/${id("chg", 1)}`,
      serve(change(1), { [`POST ${changePath}/merge`]: merged }),
    );
    await userEvent.type(input, "merge");
    await userEvent.click(await option(/Merge this change/));
    const dialog = await screen.findByRole("alertdialog");
    expect(calls.some((c) => c.path.endsWith("/merge"))).toBe(false);
    await userEvent.click(within(dialog).getByRole("button", { name: "Merge" }));
    await waitFor(() => expect(calls.some((c) => c.path.endsWith("/merge"))).toBe(true));
    expect(await screen.findByText("Change merged")).toBeInTheDocument();
  });

  it("lists Merge with its reason and does not run it while a rule is unmet", async () => {
    const { calls, input } = await palette(
      `/changes/${id("chg", 1)}`,
      serve(change(1, { readiness: readiness({ checks: "running" }) })),
    );
    await userEvent.type(input, "merge");
    const merge = await option(/Merge this change/);
    expect(merge).toHaveAttribute("aria-disabled", "true");
    expect(merge).toHaveTextContent("Checks are still running");
    await userEvent.click(merge);
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(calls.some((c) => c.method === "POST")).toBe(false);
  });

  it("approves the open change", async () => {
    const { calls, input } = await palette(
      `/changes/${id("chg", 1)}`,
      serve(change(1), { [`POST ${changePath}/approve`]: change(1) }),
    );
    await userEvent.type(input, "approve");
    await userEvent.keyboard("{Enter}");
    await waitFor(() =>
      expect(calls.find((c) => c.path.endsWith("/approve"))?.body).toEqual({ revision: 1 }),
    );
  });

  it("closes the open change after a confirmation", async () => {
    const { calls, input } = await palette(
      `/changes/${id("chg", 1)}`,
      serve(change(1), { [`POST ${changePath}/close`]: change(1, { phase: "closed" }) }),
    );
    await userEvent.type(input, "close this");
    await userEvent.keyboard("{Enter}");
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Close change" }),
    );
    await waitFor(() => expect(calls.some((c) => c.path.endsWith("/close"))).toBe(true));
  });

  it("offers no change command away from a change", async () => {
    const { input } = await palette("/");
    await userEvent.type(input, "merge");
    expect(screen.queryByRole("option", { name: /Merge this change/ })).not.toBeInTheDocument();
  });

  it("cancels the open task", async () => {
    const { calls, input } = await palette(`/tasks/${id("task", 1)}`, {
      [`GET ${taskPath}`]: task(1),
      [`GET ${taskPath}/transcript`]: transcript([]),
      [`POST ${taskPath}/cancel`]: task(1, { phase: "cancelled" }),
    });
    await userEvent.type(input, "cancel");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(calls.some((c) => c.path.endsWith("/cancel"))).toBe(true));
    expect(await screen.findByText("Task cancelled")).toBeInTheDocument();
  });

  it("drops a page's commands when the page goes", async () => {
    const { router } = await palette(`/tasks/${id("task", 1)}`, {
      [`GET ${taskPath}`]: task(1),
      [`GET ${taskPath}/transcript`]: transcript([]),
    });
    expect(await option(/Cancel this task/)).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    await router.navigate({ to: "/system", search: { sandbox: undefined } });
    await userEvent.keyboard("{Control>}k{/Control}");
    await screen.findByRole("combobox");
    expect(screen.queryByRole("option", { name: /Cancel this task/ })).not.toBeInTheDocument();
  });
});
