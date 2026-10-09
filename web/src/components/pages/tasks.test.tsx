import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { change, id, task, transcript } from "@/test/fixtures";
import { REPO, renderApp, stubServer } from "@/test/render-app";
import { signIn, TOKEN } from "@/test/server";

afterEach(() => {
  vi.unstubAllGlobals();
});

const tasksPath = `GET /v1/repos/${REPO.id}/tasks`;
const transcriptPath = (n: number) => `GET /v1/tasks/${id("task", n)}/transcript`;
const taskPath = (n: number) => `GET /v1/tasks/${id("task", n)}`;

describe("Tasks", () => {
  it("lists tasks by state, what needs the user first", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [tasksPath]: [
        task(1, { phase: "done", goal: "Old and done" }),
        task(2, { phase: "awaiting_review", goal: "Ready for you" }),
        task(3, { phase: "failed", goal: "Broke", error: "sandbox lost" }),
        task(4, { phase: "working", goal: "In progress" }),
        task(5, { phase: "cancelled", goal: "Dropped" }),
      ],
    });
    renderApp("/tasks");
    await screen.findByText("Ready for you");
    const headings = screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent);
    expect(headings.map((h) => h?.replace(/\d+$/, ""))).toEqual([
      "Needs you",
      "Running",
      "Failed",
      "Passed",
      "Closed",
    ]);
    expect(screen.getByText("sandbox lost")).toBeInTheDocument();
    expect(screen.getByText("Working, turn 1")).toBeInTheDocument();
  });

  it("says when there are no tasks", async () => {
    signIn();
    stubServer(TOKEN, [REPO], { [tasksPath]: [] });
    renderApp("/tasks");
    expect(await screen.findByText(/No tasks yet/)).toBeInTheDocument();
  });

  it("creates a task and opens it", async () => {
    signIn();
    const created = task(9, { goal: "Fix the build", phase: "preparing", turns: [] });
    const { calls } = stubServer(TOKEN, [REPO], {
      [tasksPath]: [],
      [`POST /v1/repos/${REPO.id}/tasks`]: created,
      [taskPath(9)]: created,
      [transcriptPath(9)]: transcript([]),
    });
    const { router } = renderApp("/tasks");
    await userEvent.click(await screen.findByRole("button", { name: "Create task" }));
    const form = screen.getByRole("form", { name: "Create task" });
    await userEvent.type(within(form).getByLabelText("Goal"), "Fix the build");
    expect(within(form).getByText(/igloo task create "Fix the build"/)).toBeInTheDocument();
    await userEvent.click(within(form).getByRole("button", { name: "Create" }));
    await waitFor(() => expect(router.state.location.pathname).toBe(`/tasks/${created.id}`));
    expect(calls.find((call) => call.method === "POST")?.body).toEqual({
      goal: "Fix the build",
      tool: null,
    });
    expect(await screen.findByText("Preparing the sandbox")).toBeInTheDocument();
  });
});

const call = (n: number, name: string, input: unknown) => ({
  kind: "tool_call" as const,
  id: `toolu_${n}`,
  name,
  input: JSON.stringify(input),
});
const result = (n: number, output: string, is_error = false) => ({
  kind: "tool_result" as const,
  id: `toolu_${n}`,
  output,
  is_error,
});

describe("Task", () => {
  it("shows the transcript with tool calls collapsed and expandable", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [taskPath(1)]: task(1),
      [transcriptPath(1)]: transcript([
        { kind: "message", text: "I will look at the build." },
        call(1, "Bash", { command: "cargo build\n--release" }),
        result(1, "error[E0432]: unresolved import", true),
        call(2, "Edit", {
          file_path: "src/lib.rs",
          old_string: "let a = 1;",
          new_string: "let a = 2;",
        }),
        result(2, "ok"),
        { kind: "error", text: "tool crashed" },
      ]),
    });
    renderApp(`/tasks/${id("task", 1)}`);
    await screen.findByText("I will look at the build.");
    const bash = screen.getByRole("button", { name: /Bash\s+cargo build/ });
    expect(bash).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText(/unresolved import/)).not.toBeInTheDocument();
    expect(within(bash).getByText("failed")).toBeInTheDocument();

    await userEvent.click(bash);
    expect(screen.getByText(/unresolved import/)).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /Edit\s+src\/lib.rs/ }));
    expect(screen.getByText("- let a = 1;")).toBeInTheDocument();
    expect(screen.getByText("+ let a = 2;")).toBeInTheDocument();
    expect(screen.getByText("tool crashed")).toBeInTheDocument();
  });

  it("grows without a reload as entries arrive", async () => {
    signIn();
    const reads: string[] = [];
    let entries = transcript([
      { kind: "message", text: "first" },
      { kind: "message", text: "second" },
    ]);
    const { events } = stubServer(TOKEN, [REPO], {
      [taskPath(1)]: task(1),
      [transcriptPath(1)]: (request: Request) => {
        reads.push(new URL(request.url).search);
        const after = Number(new URL(request.url).searchParams.get("after") ?? 0);
        return { ...entries, entries: entries.entries.filter((e) => e.position >= after) };
      },
    });
    renderApp(`/tasks/${id("task", 1)}`);
    await screen.findByText("second");

    entries = transcript([
      { kind: "message", text: "first" },
      { kind: "message", text: "second" },
      { kind: "message", text: "third" },
    ]);
    await waitFor(() => expect(events.connections).toHaveLength(1));
    events.push(1, "task.turn_started", "task", id("task", 1), REPO.id);
    expect(await screen.findByText("third")).toBeInTheDocument();
    expect(screen.getByText("first")).toBeInTheDocument();
    expect(reads).toEqual(["?after=0", "?after=2"]);
  });

  it("reads a working task's transcript again on its own", async () => {
    signIn();
    let entries = transcript([{ kind: "message", text: "first" }]);
    stubServer(TOKEN, [REPO], {
      [taskPath(1)]: task(1),
      [transcriptPath(1)]: (request: Request) => {
        const after = Number(new URL(request.url).searchParams.get("after") ?? 0);
        return { ...entries, entries: entries.entries.filter((e) => e.position >= after) };
      },
    });
    renderApp(`/tasks/${id("task", 1)}`);
    await screen.findByText("first");
    entries = transcript([
      { kind: "message", text: "first" },
      { kind: "message", text: "polled" },
    ]);
    expect(await screen.findByText("polled", {}, { timeout: 4_000 })).toBeInTheDocument();
  });

  it("does not poll a task that is idle", async () => {
    signIn();
    const { calls } = stubServer(TOKEN, [REPO], {
      [taskPath(1)]: task(1, { phase: "done" }),
      [transcriptPath(1)]: transcript([{ kind: "message", text: "done" }], { idle: true }),
    });
    renderApp(`/tasks/${id("task", 1)}`);
    await screen.findByText("done");
    await new Promise((resolve) => setTimeout(resolve, 2_300));
    expect(calls.filter((c) => c.path.endsWith("/transcript"))).toHaveLength(1);
  });

  it("shows the failure, the change and a link to the sandbox", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [taskPath(2)]: task(2, {
        phase: "failed",
        error: "the tool exited 1",
        change: id("chg", 2),
      }),
      [`GET /v1/changes/${id("chg", 2)}`]: change(2, { title: "The fix" }),
      [transcriptPath(2)]: transcript([], { idle: true }),
    });
    renderApp(`/tasks/${id("task", 2)}`);
    expect(await screen.findByRole("alert")).toHaveTextContent("the tool exited 1");
    expect(await screen.findByRole("link", { name: /The fix/ })).toHaveAttribute(
      "href",
      `/changes/${id("chg", 2)}`,
    );
    expect(screen.getByRole("link", { name: /sbx_/ })).toHaveAttribute(
      "href",
      expect.stringContaining(`/system?sandbox=${id("sbx", 2)}`),
    );
    expect(screen.getByText("The tool printed nothing.")).toBeInTheDocument();
  });

  it("cancels a running task and shows the CLI command", async () => {
    signIn();
    const cancelled = task(1, { phase: "cancelled" });
    const { calls } = stubServer(TOKEN, [REPO], {
      [taskPath(1)]: task(1),
      [transcriptPath(1)]: transcript([]),
      [`POST /v1/tasks/${id("task", 1)}/cancel`]: cancelled,
    });
    renderApp(`/tasks/${id("task", 1)}`);
    await userEvent.click(await screen.findByRole("button", { name: "Terminal commands" }));
    expect(await screen.findByText(`igloo task cancel ${id("task", 1)}`)).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    await userEvent.click(screen.getByRole("button", { name: "Cancel task" }));
    await waitFor(() => expect(calls.some((c) => c.path.endsWith("/cancel"))).toBe(true));
    expect(await screen.findByText("Closed")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel task" })).toBeDisabled();
    expect(screen.getByText("The task already cancelled")).toBeInTheDocument();
  });
});
