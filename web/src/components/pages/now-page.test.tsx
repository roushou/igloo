import { screen, waitFor, within } from "@testing-library/react";
import { vi } from "vitest";
import { change, readiness, run, task } from "@/test/fixtures";
import { REPO, renderApp, stubServer } from "@/test/render-app";
import { signIn, TOKEN } from "@/test/server";

afterEach(() => {
  vi.unstubAllGlobals();
});

const tasksPath = `GET /v1/repos/${REPO.id}/tasks`;
const changesPath = `GET /v1/repos/${REPO.id}/changes`;
const runsPath = `GET /v1/repos/${REPO.id}/runs`;

describe("Now", () => {
  it("puts what waits on the user before what runs", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [tasksPath]: [
        task(1, { phase: "awaiting_review", goal: "Review me" }),
        task(2, { phase: "working", goal: "Busy task" }),
      ],
      [changesPath]: [
        change(3, { title: "Ready to merge" }),
        change(4, {
          title: "Touches protected paths",
          readiness: readiness({
            checks: "running",
            approval: { state: "required", protected_paths: ["proto/a.proto"] },
          }),
        }),
        change(5, { title: "Still checking", readiness: readiness({ checks: "running" }) }),
      ],
      [runsPath]: { items: [run(2)] },
    });
    renderApp("/");

    const needs = await screen.findByRole("region", { name: "Needs you" });
    expect(await within(needs).findByText("Review me")).toBeInTheDocument();
    expect(within(needs).getByText("Ready to merge")).toBeInTheDocument();
    expect(within(needs).getByText("Touches protected paths")).toBeInTheDocument();
    expect(within(needs).getByText(/touches proto\/a.proto/)).toBeInTheDocument();
    expect(within(needs).queryByText("Still checking")).not.toBeInTheDocument();
    expect(within(needs).queryByText("Busy task")).not.toBeInTheDocument();

    const running = screen.getByRole("region", { name: "Running" });
    expect(await within(running).findByText("Busy task")).toBeInTheDocument();
    expect(within(running).getByText("Working, turn 1")).toBeInTheDocument();
    expect(within(running).getByText(/Checking test \(1 of 2 checks done\)/)).toBeInTheDocument();

    expect(needs.compareDocumentPosition(running) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("says when nothing waits or runs", async () => {
    signIn();
    stubServer(TOKEN, [REPO], { [tasksPath]: [], [changesPath]: [], [runsPath]: { items: [] } });
    renderApp("/");
    expect(await screen.findByText("Nothing is waiting on you.")).toBeInTheDocument();
    expect(screen.getByText("Nothing is running.")).toBeInTheDocument();
    expect(screen.getByText("Events appear here as they happen.")).toBeInTheDocument();
  });

  it("shows an errored run's reason where it is listed", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [tasksPath]: [],
      [changesPath]: [],
      [runsPath]: {
        items: [run(7, { phase: "errored", error: "the worker went away", checks: [] })],
      },
    });
    renderApp("/");
    const recent = await screen.findByRole("region", { name: "Recent runs" });
    expect(await within(recent).findByText("the worker went away")).toBeInTheDocument();
    expect(within(recent).getByText("Errored")).toBeInTheDocument();
  });

  it("updates without a reload and lists the event", async () => {
    signIn();
    let tasks = [task(2, { phase: "working", goal: "Busy task" })];
    const { events } = stubServer(TOKEN, [REPO], {
      [tasksPath]: () => tasks,
      [changesPath]: [],
      [runsPath]: { items: [] },
    });
    renderApp("/");
    const needs = await screen.findByRole("region", { name: "Needs you" });
    await screen.findByText("Busy task");
    expect(within(needs).queryByText("Busy task")).not.toBeInTheDocument();

    tasks = [task(2, { phase: "awaiting_review", goal: "Busy task" })];
    await waitFor(() => expect(events.connections).toHaveLength(1));
    events.push(1, "task.awaiting_review", "task", tasks[0]?.id ?? "", REPO.id);

    expect(await within(needs).findByText("Busy task")).toBeInTheDocument();
    const activity = screen.getByRole("list", { name: "Recent activity" });
    expect(within(activity).getByText("task awaiting review")).toBeInTheDocument();
  });

  it("tells when the connection is lost and when it returns", async () => {
    signIn();
    const { events } = stubServer(TOKEN, [REPO], {
      [tasksPath]: [],
      [changesPath]: [],
      [runsPath]: { items: [] },
    });
    renderApp("/");
    expect(await screen.findByText("Live")).toBeInTheDocument();
    events.drop();
    expect(await screen.findByText("Reconnecting…")).toBeInTheDocument();
  });
});
