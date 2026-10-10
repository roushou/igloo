import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { id, me, task, transcript } from "@/test/fixtures";
import { REPO, renderApp, stubServer } from "@/test/render-app";
import { signIn, TOKEN } from "@/test/server";

/** The terminal records the sandbox and mode the page asks for; its own tests cover the rest. */
vi.mock("@/components/terminal", () => ({
  Terminal: ({ sandboxId, mode }: { sandboxId: string; mode?: string }) => (
    <div role="application" aria-label="Terminal" data-sandbox={sandboxId} data-mode={mode} />
  ),
}));

afterEach(() => {
  vi.unstubAllGlobals();
});

const taskPath = `GET /v1/tasks/${id("task", 1)}`;
const transcriptPath = `GET /v1/tasks/${id("task", 1)}/transcript`;
const takeOverPath = `POST /v1/tasks/${id("task", 1)}/take-over`;
const handBackPath = `POST /v1/tasks/${id("task", 1)}/hand-back`;
const mePath = "GET /v1/me";
const by = "usr_00000000000000000000000007";

const open = () => renderApp(`/tasks/${id("task", 1)}`);

describe("Task sandbox", () => {
  it("watches the sandbox read-only, on request", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [mePath]: me(),
      [taskPath]: task(1, { phase: "awaiting_review" }),
      [transcriptPath]: transcript([], { idle: true }),
    });
    open();
    await userEvent.click(await screen.findByRole("button", { name: "Watch the sandbox" }));
    const terminal = screen.getByRole("application", { name: "Terminal" });
    expect(terminal).toHaveAttribute("data-mode", "read_only");
    expect(terminal).toHaveAttribute("data-sandbox", id("sbx", 1));
    expect(screen.getByText(/Read-only: what you type is not sent/)).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Hide terminal" }));
    expect(screen.queryByRole("application", { name: "Terminal" })).not.toBeInTheDocument();
  });

  it("takes the task over, types, waits for the turn, and hands it back", async () => {
    signIn();
    let current = task(1);
    const { calls } = stubServer(TOKEN, [REPO], {
      [mePath]: me(),
      [taskPath]: () => current,
      [transcriptPath]: transcript([]),
      [takeOverPath]: () => {
        current = task(1, { takeover: { by, phase: "waiting" } });
        return current;
      },
      [handBackPath]: () => {
        current = task(1, { takeover: { by, phase: "handing_back" } });
        return current;
      },
    });
    open();
    await userEvent.click(await screen.findByRole("button", { name: "Take over" }));
    await waitFor(() => expect(calls.some((call) => call.path.endsWith("/take-over"))).toBe(true));

    // The turn is still running: the terminal is writable, handing back waits.
    expect(await screen.findByText(/Its current turn is finishing/)).toBeInTheDocument();
    expect(await screen.findByRole("application", { name: "Terminal" })).toHaveAttribute(
      "data-mode",
      "read_write",
    );
    expect(screen.getByText("Your keystrokes go to the sandbox.")).toBeInTheDocument();
    expect(screen.getByText("Taken over, the current turn is finishing")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Hand back" })).toBeDisabled();
    expect(
      screen.getByText("The current turn is still running; hand back once it finishes"),
    ).toBeInTheDocument();
  });

  it("hands back a paused task and goes back to watching", async () => {
    signIn();
    let current = task(1, { phase: "awaiting_review", takeover: { by, phase: "paused" } });
    const { calls } = stubServer(TOKEN, [REPO], {
      [mePath]: me(),
      [taskPath]: () => current,
      [transcriptPath]: transcript([], { idle: true }),
      [handBackPath]: () => {
        current = task(1, { takeover: { by, phase: "handing_back" } });
        return current;
      },
    });
    open();
    expect(await screen.findByText(/It is paused until you hand it back/)).toBeInTheDocument();
    expect(await screen.findByRole("application", { name: "Terminal" })).toHaveAttribute(
      "data-mode",
      "read_write",
    );
    await userEvent.click(screen.getByRole("button", { name: "Hand back" }));
    await waitFor(() => expect(calls.some((call) => call.path.endsWith("/hand-back"))).toBe(true));

    expect(await screen.findByText(/reading what you changed/)).toBeInTheDocument();
    expect(await screen.findByRole("application", { name: "Terminal" })).toHaveAttribute(
      "data-mode",
      "read_only",
    );
    expect(screen.getByRole("button", { name: "Hand back" })).toBeDisabled();
  });

  it("tells the viewer another person holds the take-over, keeps the terminal read-only and lets them hand back", async () => {
    signIn();
    const other = "usr_00000000000000000000000009";
    let current = task(1, { phase: "awaiting_review", takeover: { by: other, phase: "paused" } });
    const { calls } = stubServer(TOKEN, [REPO], {
      [mePath]: me(),
      [taskPath]: () => current,
      [transcriptPath]: transcript([], { idle: true }),
      [handBackPath]: () => {
        current = task(1, { takeover: { by: other, phase: "handing_back" } });
        return current;
      },
    });
    open();
    expect(
      await screen.findByText(/took the task over, so the terminal is read-only for you/),
    ).toBeInTheDocument();
    expect(await screen.findByRole("application", { name: "Terminal" })).toHaveAttribute(
      "data-mode",
      "read_only",
    );
    expect(screen.getByText(/only the person who took the task over can type/)).toBeInTheDocument();
    expect(screen.queryByText("Your keystrokes go to the sandbox.")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Hand back" }));
    await waitFor(() => expect(calls.some((call) => call.path.endsWith("/hand-back"))).toBe(true));
  });

  it("types in the terminal of a take-over that is the viewer\x27s", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [mePath]: me({ id: by }),
      [taskPath]: task(1, { phase: "awaiting_review", takeover: { by, phase: "paused" } }),
      [transcriptPath]: transcript([], { idle: true }),
    });
    open();
    expect(await screen.findByText(/You took the task over/)).toBeInTheDocument();
    expect(await screen.findByRole("application", { name: "Terminal" })).toHaveAttribute(
      "data-mode",
      "read_write",
    );
  });

  it("shows the same from a terminal", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [mePath]: me(),
      [taskPath]: task(1, { phase: "awaiting_review", takeover: { by, phase: "paused" } }),
      [transcriptPath]: transcript([], { idle: true }),
    });
    open();
    await userEvent.click(await screen.findByRole("button", { name: "Terminal commands" }));
    expect(await screen.findByText(`igloo shell ${id("task", 1)}`)).toBeInTheDocument();
    expect(screen.getByText(`igloo task hand-back ${id("task", 1)}`)).toBeInTheDocument();
  });

  it("has no sandbox to show once the task ended, and cannot be taken over", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [mePath]: me(),
      [taskPath]: task(1, { phase: "done" }),
      [transcriptPath]: transcript([], { idle: true }),
    });
    open();
    expect(await screen.findByRole("button", { name: "Take over" })).toBeDisabled();
    expect(screen.queryByRole("region", { name: "Sandbox" })).not.toBeInTheDocument();
  });

  it("says why the server refused", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [mePath]: me(),
      [taskPath]: task(1, { phase: "awaiting_review" }),
      [transcriptPath]: transcript([], { idle: true }),
      [takeOverPath]: Response.json(
        {
          title: "Conflicts with the current state",
          status: 409,
          code: "task.already_taken_over",
          type: "about:blank",
          detail: "the task is taken over by someone else",
        },
        { status: 409 },
      ),
    });
    open();
    await userEvent.click(await screen.findByRole("button", { name: "Take over" }));
    expect(await screen.findByText("Could not take the task over")).toBeInTheDocument();
  });
});
