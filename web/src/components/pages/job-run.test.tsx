import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { format } from "@/lib/format";
import { id, job, run } from "@/test/fixtures";
import { REPO, renderApp, StubEvents, stubServer } from "@/test/render-app";
import { signIn, TOKEN } from "@/test/server";

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const jobPath = (n: number) => `GET /v1/jobs/${id("job", n)}`;
const logsPath = (n: number) => `GET /v1/jobs/${id("job", n)}/logs`;

function frame(sequence: number, event: string, data: string): string {
  return `id: ${sequence}\nevent: ${event}\n${data
    .split("\n")
    .map((line) => `data: ${line}`)
    .join("\n")}\n\n`;
}

/** A server with job `n` and a log stream the test feeds. */
function serveLog(n: number, jobBody = job(n)) {
  const logs = new StubEvents();
  const server = stubServer(TOKEN, [REPO], {
    [jobPath(n)]: jobBody,
    [logsPath(n)]: (request: Request) => logs.open(request.headers.get("Last-Event-ID")),
  });
  return { logs, ...server };
}

describe("Job log", () => {
  it("follows the output while the job runs and ends with the job", async () => {
    signIn();
    const { logs } = serveLog(1);
    renderApp(`/jobs/${id("job", 1)}`);
    expect(await screen.findByText("Waiting for output…")).toBeInTheDocument();
    await waitFor(() => expect(logs.connections).toHaveLength(1));

    logs.raw(frame(1, "stdout", "Compiling igloo\n"));
    expect(await screen.findByText("Compiling igloo")).toBeInTheDocument();
    expect(screen.getByText("Following")).toBeInTheDocument();

    logs.raw(frame(2, "stderr", "warning: unused\n"));
    logs.raw(frame(3, "stdout", "Finished"));
    expect(await screen.findByText("warning: unused")).toBeInTheDocument();
    // An unterminated last line is shown while it is the latest output.
    expect(screen.getByText("Finished")).toBeInTheDocument();

    logs.raw(frame(4, "end", JSON.stringify(job(1, { phase: "finished", exit_code: 0 }))));
    expect(await screen.findByText("Finished", { selector: "[role=status]" })).toBeInTheDocument();
  });

  it("resumes after the last event when the connection drops", async () => {
    signIn();
    const { logs } = serveLog(1);
    renderApp(`/jobs/${id("job", 1)}`);
    await waitFor(() => expect(logs.connections).toHaveLength(1));
    logs.raw(frame(7, "stdout", "one\n"));
    await screen.findByText("one");
    logs.drop();
    await waitFor(() => expect(logs.connections).toEqual([null, "7"]), { timeout: 3_000 });
    logs.raw(frame(8, "stdout", "two\n"));
    expect(await screen.findByText("two")).toBeInTheDocument();
    expect(screen.getAllByText("one")).toHaveLength(1);
  });

  it("searches the lines and steps through the matches", async () => {
    signIn();
    const { logs } = serveLog(1, job(1, { phase: "finished", exit_code: 0 }));
    renderApp(`/jobs/${id("job", 1)}`);
    await waitFor(() => expect(logs.connections).toHaveLength(1));
    logs.raw(frame(1, "stdout", "alpha\nbeta\nalpha again\n"));
    logs.raw(frame(2, "end", JSON.stringify(job(1, { phase: "finished", exit_code: 0 }))));
    await screen.findByText("beta");

    await userEvent.type(screen.getByLabelText("Search the log"), "alpha");
    expect(screen.getByText("2 matches")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(document.querySelector("[data-highlighted]")?.textContent).toMatch(/1alpha$/);
    await userEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(document.querySelector("[data-highlighted]")).toHaveTextContent("alpha again");
  });

  it("jumps to the first line that mentions an error", async () => {
    signIn();
    const { logs } = serveLog(1);
    renderApp(`/jobs/${id("job", 1)}`);
    await waitFor(() => expect(logs.connections).toHaveLength(1));
    logs.raw(frame(1, "stdout", "ok one\nerror[E0432]: unresolved import\nok two\nERROR again\n"));
    await screen.findByText("ok two");
    await userEvent.click(screen.getByRole("button", { name: "Jump to first error" }));
    const highlighted = document.querySelector("[data-highlighted]");
    expect(highlighted).toHaveTextContent("error[E0432]: unresolved import");
    expect(screen.getByRole("checkbox", { name: "Follow" })).not.toBeChecked();
  });

  it("windows a long log instead of rendering every line", async () => {
    signIn();
    vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(400);
    vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(800);
    const { logs } = serveLog(1, job(1, { phase: "finished", exit_code: 0 }));
    renderApp(`/jobs/${id("job", 1)}`);
    await waitFor(() => expect(logs.connections).toHaveLength(1));
    const lines = Array.from({ length: 5000 }, (_, i) => `line ${i}`).join("\n");
    logs.raw(frame(1, "stdout", `${lines}\n`));
    await waitFor(() =>
      expect(document.querySelectorAll("[data-index]").length).toBeGreaterThan(0),
    );
    expect(document.querySelectorAll("[data-index]").length).toBeLessThan(500);
  });

  it("shows the reason a job failed", async () => {
    signIn();
    serveLog(1, job(1, { phase: "failed", failure_reason: "the worker lost the sandbox" }));
    renderApp(`/jobs/${id("job", 1)}`);
    expect(await screen.findByRole("alert")).toHaveTextContent("the worker lost the sandbox");
    expect(screen.getByText("Errored")).toBeInTheDocument();
  });
});

describe("Run", () => {
  it("lists the checks with their logs and the reason of an error", async () => {
    signIn();
    const errored = run(1, {
      phase: "errored",
      error: "no worker could take the warm-up",
      warm_job: id("job", 50),
      checks: [
        { name: "lint", status: "passed", job: id("job", 101) },
        { name: "test", status: "failed", job: id("job", 102), exit_code: 1 },
        { name: "doc", status: "errored", reason: "the sandbox was lost" },
      ],
    });
    stubServer(TOKEN, [REPO], { [`GET /v1/runs/${errored.id}`]: errored });
    renderApp(`/runs/${errored.id}`);
    expect(await screen.findByRole("alert")).toHaveTextContent("no worker could take the warm-up");
    const checks = screen.getByRole("list", { name: "Checks" });
    expect(within(checks).getByText("Passed")).toBeInTheDocument();
    expect(within(checks).getByText("Failed")).toBeInTheDocument();
    expect(within(checks).getByText("the sandbox was lost")).toBeInTheDocument();
    expect(within(checks).getAllByRole("link", { name: "Log" })[1]).toHaveAttribute(
      "href",
      `/jobs/${id("job", 102)}`,
    );
    expect(screen.getByRole("link", { name: "Warm-up log" })).toBeInTheDocument();
  });

  it("shows when a run ended, its sandbox, and how long each check took or has run", async () => {
    signIn();
    const finished = run(3, {
      phase: "failed",
      change: id("chg", 3),
      started_at: "2026-01-01T10:00:00Z",
      ended_at: "2026-01-01T10:05:00Z",
      sandbox: id("sbx", 3),
      checks: [
        {
          name: "lint",
          status: "passed",
          job: id("job", 1),
          started_at: "2026-01-01T10:00:00Z",
          ended_at: "2026-01-01T10:00:42Z",
        },
        {
          name: "test",
          status: "started",
          job: id("job", 2),
          started_at: new Date().toISOString(),
        },
      ],
    });
    const before = run(2, {
      phase: "passed",
      change: id("chg", 3),
      started_at: "2026-01-01T09:00:00Z",
      checks: [
        {
          name: "test",
          status: "passed",
          job: id("job", 5),
          started_at: "2026-01-01T09:00:00Z",
          ended_at: "2026-01-01T09:03:05Z",
        },
      ],
    });
    stubServer(TOKEN, [REPO], {
      [`GET /v1/runs/${finished.id}`]: finished,
      [`GET /v1/changes/${id("chg", 3)}/runs`]: [before, finished],
    });
    renderApp(`/runs/${finished.id}`);
    const checks = await screen.findByRole("list", { name: "Checks" });
    expect(await within(checks).findByText("42s")).toBeInTheDocument();
    expect(await within(checks).findByText(/previously 3m 05s/)).toBeInTheDocument();
    expect(screen.getByText("Ended")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: format.shortId(id("sbx", 3)) })).toBeInTheDocument();
  });

  it("shows a running run's step", async () => {
    signIn();
    const running = run(2);
    stubServer(TOKEN, [REPO], { [`GET /v1/runs/${running.id}`]: running });
    renderApp(`/runs/${running.id}`);
    expect(await screen.findByText(/Checking test \(1 of 2 checks done\)/)).toBeInTheDocument();
    expect(screen.getAllByText("Running", { selector: "[data-state]" })).toHaveLength(2);
  });
});
