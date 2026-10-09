import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { api } from "@/api/client";
import { REPO, renderApp, stubServer } from "@/test/render-app";

const TOKEN = "right-token";

afterEach(() => {
  vi.unstubAllGlobals();
});

async function signIn(token: string) {
  await userEvent.type(await screen.findByLabelText("API token"), token);
  await userEvent.click(screen.getByRole("button", { name: "Sign in" }));
}

describe("sign-in", () => {
  it("sends a visitor without a token to the sign-in page", async () => {
    stubServer(TOKEN);
    const { router } = renderApp("/tasks");
    await screen.findByRole("heading", { name: "Sign in to Igloo" });
    expect(router.state.location.pathname).toBe("/signin");
  });

  it("stays on the page with an error for a wrong token", async () => {
    stubServer(TOKEN);
    const { router } = renderApp("/signin");
    await signIn("wrong-token");
    expect(await screen.findByRole("alert")).toHaveTextContent("rejected");
    expect(router.state.location.pathname).toBe("/signin");
    expect(api.tokens.get()).toBeNull();
  });

  it("opens the shell for the right token and keeps it", async () => {
    stubServer(TOKEN);
    const { router } = renderApp("/signin");
    await signIn(TOKEN);
    await screen.findByRole("navigation", { name: "Main" });
    expect(router.state.location.pathname).toBe("/");
    expect(api.tokens.get()).toBe(TOKEN);
    expect(await screen.findByRole("combobox", { name: "Repository" })).toHaveValue(REPO.id);
  });

  it("reports an unreachable server", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("network")));
    renderApp("/signin");
    await signIn(TOKEN);
    expect(await screen.findByRole("alert")).toHaveTextContent("could not be reached");
  });

  it("returns to sign-in when any request is answered with 401", async () => {
    api.tokens.set("expired-token");
    stubServer(TOKEN);
    const { router } = renderApp("/");
    await waitFor(() => expect(router.state.location.pathname).toBe("/signin"));
    expect(api.tokens.get()).toBeNull();
  });
});

describe("shell", () => {
  it("links the pages and offers a Sign out", async () => {
    api.tokens.set(TOKEN);
    stubServer(TOKEN);
    renderApp("/");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    for (const name of ["Now", "Tasks", "Changes", "System", "Settings"]) {
      expect(nav).toHaveTextContent(name);
    }
    await userEvent.click(screen.getByRole("button", { name: "Sign out" }));
    await screen.findByRole("heading", { name: "Sign in to Igloo" });
    expect(api.tokens.get()).toBeNull();
  });

  it("switches repositories", async () => {
    api.tokens.set(TOKEN);
    const other = { ...REPO, id: "repo_01j9z3k4m5n6p7q8r9s0t1v2w4", location: "https://x/y" };
    stubServer(TOKEN, [REPO, other]);
    renderApp("/");
    const select = await screen.findByRole("combobox", { name: "Repository" });
    await userEvent.selectOptions(select, other.id);
    expect(select).toHaveValue(other.id);
    expect(localStorage.getItem("igloo.repo")).toBe(other.id);
  });
});

describe("command palette", () => {
  it.each([
    ["task_01j9z3k4m5n6p7q8r9s0t1v2w3", "/tasks/task_01j9z3k4m5n6p7q8r9s0t1v2w3"],
    ["chg_01j9z3k4m5n6p7q8r9s0t1v2w3", "/changes/chg_01j9z3k4m5n6p7q8r9s0t1v2w3"],
    ["run_01j9z3k4m5n6p7q8r9s0t1v2w3", "/runs/run_01j9z3k4m5n6p7q8r9s0t1v2w3"],
    ["job_01j9z3k4m5n6p7q8r9s0t1v2w3", "/jobs/job_01j9z3k4m5n6p7q8r9s0t1v2w3"],
    ["sbx_01j9z3k4m5n6p7q8r9s0t1v2w3", "/system"],
  ])("opens %s with ⌘K", async (id, path) => {
    api.tokens.set(TOKEN);
    stubServer(TOKEN);
    const { router } = renderApp("/");
    await screen.findByRole("navigation", { name: "Main" });
    await userEvent.keyboard("{Meta>}k{/Meta}");
    await userEvent.paste(id);
    await userEvent.click(await within(await screen.findByRole("dialog")).findByRole("option"));
    await waitFor(() => expect(router.state.location.pathname).toBe(path));
  });

  it("selects a pasted repository id", async () => {
    api.tokens.set(TOKEN);
    stubServer(TOKEN);
    renderApp("/tasks");
    await screen.findByRole("navigation", { name: "Main" });
    await screen.findByRole("combobox", { name: "Repository" });
    await userEvent.keyboard("{Control>}k{/Control}");
    await userEvent.paste(REPO.id);
    await userEvent.click(await within(await screen.findByRole("dialog")).findByRole("option"));
    await waitFor(() => expect(localStorage.getItem("igloo.repo")).toBe(REPO.id));
  });

  it("offers nothing for text that is not an id", async () => {
    api.tokens.set(TOKEN);
    stubServer(TOKEN);
    renderApp("/");
    await screen.findByRole("navigation", { name: "Main" });
    await userEvent.keyboard("{Meta>}k{/Meta}");
    await userEvent.paste("hello");
    expect(await screen.findByText("Paste a resource id to open it.")).toBeInTheDocument();
  });
});
