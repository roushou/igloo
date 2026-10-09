import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { vi } from "vitest";
import { change, diff, file, id, job, readiness, run } from "@/test/fixtures";
import { REPO, renderApp, StubEvents, stubServer } from "@/test/render-app";
import { signIn, TOKEN } from "@/test/server";

// The diff library draws into shadow DOM, which the test DOM cannot show. This stand-in renders
// the patch text, the line annotations and the gutter button the page gives it.
vi.mock("@pierre/diffs/react", () => ({
  PatchDiff: (props: {
    patch: string;
    lineAnnotations?: { lineNumber: number; metadata: unknown }[];
    renderAnnotation?: (annotation: { lineNumber: number; metadata: unknown }) => ReactNode;
    renderGutterUtility?: (hovered: () => { lineNumber: number; side: string }) => ReactNode;
  }) => (
    <div>
      <pre>{props.patch}</pre>
      {props.lineAnnotations?.map((annotation) => (
        <div key={JSON.stringify(annotation)} data-line={annotation.lineNumber}>
          {props.renderAnnotation?.(annotation)}
        </div>
      ))}
      {props.renderGutterUtility?.(() => ({ lineNumber: 2, side: "additions" }))}
    </div>
  ),
}));

afterEach(() => {
  vi.unstubAllGlobals();
});

const changesPath = `GET /v1/repos/${REPO.id}/changes`;
const changePath = (n: number) => `/v1/changes/${id("chg", n)}`;

describe("Changes", () => {
  it("lists changes by state with what each waits on", async () => {
    signIn();
    stubServer(TOKEN, [REPO], {
      [changesPath]: [
        change(1, { title: "Merged one", phase: "merged" }),
        change(2, { title: "Ready one" }),
        change(3, { title: "Checking one", readiness: readiness({ checks: "running" }) }),
        change(4, { title: "Failing one", readiness: readiness({ checks: "failed" }) }),
        change(5, { title: "Closed one", phase: "closed" }),
      ],
    });
    renderApp("/changes");
    await screen.findByText("Ready one");
    const headings = screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent);
    expect(headings.map((h) => h?.replace(/\d+$/, ""))).toEqual([
      "Needs you",
      "Running",
      "Failed",
      "Passed",
      "Closed",
    ]);
    expect(screen.getByText("Checks passed: ready to merge")).toBeInTheDocument();
    expect(screen.getAllByText(/igloo\/task-2/)).toHaveLength(1);
  });

  it("says when there are no changes", async () => {
    signIn();
    stubServer(TOKEN, [REPO], { [changesPath]: [] });
    renderApp("/changes");
    expect(await screen.findByText(/No change yet/)).toBeInTheDocument();
  });
});

/** Opens the "More actions" menu and returns the item called `name`. */
async function menuItem(name: string) {
  await userEvent.click(await screen.findByRole("button", { name: "More actions" }));
  return screen.findByRole("menuitem", { name: new RegExp(name) });
}

/** Confirms the dialog a merge or a close asks for. */
async function confirmWith(name: string) {
  const dialog = await screen.findByRole("alertdialog");
  await userEvent.click(within(dialog).getByRole("button", { name }));
}

function serveChange(n: number, body = change(n), extra: Record<string, unknown> = {}) {
  return stubServer(TOKEN, [REPO], {
    [`GET ${changePath(n)}`]: body,
    [`GET ${changePath(n)}/diff`]: diff([file("src/lib.rs")]),
    [`GET ${changePath(n)}/runs`]: [],
    ...extra,
  });
}

describe("Change: merge checklist and actions", () => {
  it("enables Merge when every rule is met and merges", async () => {
    signIn();
    const merged = change(1, { phase: "merged", merged_commit: "c".repeat(40) });
    const { calls } = serveChange(1, change(1), { [`POST ${changePath(1)}/merge`]: merged });
    renderApp(`/changes/${id("chg", 1)}`);
    const checklist = await screen.findByRole("list", { name: "Merge checklist" });
    expect(within(checklist).getByText("Checks passed")).toBeInTheDocument();
    expect(within(checklist).getByText("No approval required")).toBeInTheDocument();
    expect(within(checklist).getByText("Up to date with main")).toBeInTheDocument();

    const merge = screen.getByRole("button", { name: "Merge" });
    expect(merge).toBeEnabled();
    await userEvent.click(merge);
    await confirmWith("Merge");
    await waitFor(() => expect(calls.some((c) => c.path.endsWith("/merge"))).toBe(true));
    expect(await screen.findByText("Merged as")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Merge" })).toBeDisabled();
    expect(screen.getAllByText("The change is already merged").length).toBeGreaterThan(0);
  });

  it.each([
    ["checks are running", { checks: "running" as const }, "Checks are still running"],
    ["a check failed", { checks: "failed" as const }, "A check failed"],
    ["no check ran", { checks: "missing" as const }, "No checks have run for the latest revision"],
    [
      "approval is needed",
      { approval: { state: "required" as const, protected_paths: ["proto/a.proto"] } },
      "A human must approve it: it touches proto/a.proto",
    ],
    [
      "the target moved",
      { fast_forward: false },
      "main has moved: record a new revision after rebasing",
    ],
  ])("disables Merge with its reason when %s", async (_, overrides, reason) => {
    signIn();
    serveChange(1, change(1, { readiness: readiness(overrides) }));
    renderApp(`/changes/${id("chg", 1)}`);
    const merge = await screen.findByRole("button", { name: "Merge" });
    expect(merge).toBeDisabled();
    expect(merge).toHaveAccessibleDescription(reason);
    const checklist = screen.getByRole("list", { name: "Merge checklist" });
    expect(within(checklist).getByText(reason)).toBeInTheDocument();
  });

  it("shows the server's reason when a merge is refused", async () => {
    signIn();
    serveChange(1, change(1), {
      [`POST ${changePath(1)}/merge`]: () =>
        Response.json(
          {
            type: "about:blank",
            title: "Conflict",
            status: 409,
            code: "change.target_moved",
            detail: "main moved since the last revision",
          },
          { status: 409 },
        ),
    });
    renderApp(`/changes/${id("chg", 1)}`);
    await userEvent.click(await screen.findByRole("button", { name: "Merge" }));
    await confirmWith("Merge");
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "main moved since the last revision",
    );
  });

  it("approves the latest revision, requests changes, records a revision and closes", async () => {
    signIn();
    const two = change(1, {
      revisions: [
        {
          number: 1,
          base: "a".repeat(40),
          head: "b".repeat(40),
          created_at: "2026-01-01T10:00:00Z",
        },
        {
          number: 2,
          base: "a".repeat(40),
          head: "d".repeat(40),
          created_at: "2026-01-01T11:00:00Z",
        },
      ],
    });
    const { calls } = serveChange(1, two, {
      [`POST ${changePath(1)}/approve`]: two,
      [`POST ${changePath(1)}/request-changes`]: two,
      [`POST ${changePath(1)}/revisions`]: two,
      [`POST ${changePath(1)}/close`]: change(1, { phase: "closed" }),
    });
    renderApp(`/changes/${id("chg", 1)}`);
    await userEvent.click(await screen.findByRole("button", { name: "Approve" }));
    await userEvent.click(await menuItem("Request changes"));
    await userEvent.click(await menuItem("Record revision"));
    await waitFor(() => expect(calls.filter((c) => c.method === "POST")).toHaveLength(3));
    expect(calls.find((c) => c.path.endsWith("/approve"))?.body).toEqual({ revision: 2 });

    await userEvent.click(await menuItem("Close"));
    await confirmWith("Close change");
    expect(await screen.findByText("Closed", { selector: "[data-state]" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(await menuItem("Request changes")).toHaveAttribute("aria-disabled", "true");
  });

  it("disables Approve once the latest revision is approved", async () => {
    signIn();
    serveChange(
      1,
      change(1, {
        readiness: readiness({ approval: { state: "given", protected_paths: ["proto/a.proto"] } }),
        approvals: [{ at: "2026-01-01T12:00:00Z", by: "usr_1", revision: 1 }],
      }),
    );
    renderApp(`/changes/${id("chg", 1)}`);
    const approve = await screen.findByRole("button", { name: "Approve" });
    expect(approve).toBeDisabled();
    expect(approve).toHaveAccessibleDescription("Revision 1 is already approved");
  });

  it.each([
    ["Record revision", `igloo change push ${id("chg", 1)}`],
    ["Request changes", `igloo change request-changes ${id("chg", 1)}`],
    ["Approve", `igloo change approve ${id("chg", 1)}`],
    ["Merge", `igloo change merge ${id("chg", 1)}`],
    ["Close", `igloo change close ${id("chg", 1)}`],
  ])("shows the CLI command of %s", async (label, command) => {
    signIn();
    serveChange(1);
    renderApp(`/changes/${id("chg", 1)}`);
    await userEvent.click(await screen.findByRole("button", { name: "Terminal commands" }));
    const popover = await screen.findByRole("dialog");
    expect(within(popover).getByText(label)).toBeInTheDocument();
    expect(within(popover).getByText(command)).toBeInTheDocument();
  });
});

describe("Change: tabs", () => {
  it("shows the files of the diff and the revision's line comments", async () => {
    signIn();
    const withComments = change(1, {
      comments: [
        {
          at: "2026-01-01T10:30:00Z",
          author: "usr_1",
          body: "Why this?",
          path: "src/lib.rs",
          line: 2,
          revision: 1,
        },
        { at: "2026-01-01T10:31:00Z", author: "usr_1", body: "General note", revision: 1 },
      ],
    });
    serveChange(1, withComments, {
      [`GET ${changePath(1)}/diff`]: diff([
        file("src/lib.rs", { status: "renamed", previous_path: "src/old.rs" }),
        file("logo.png", { binary: true, patch: null }),
        file("huge.txt", { truncated: true, patch: null }),
      ]),
    });
    renderApp(`/changes/${id("chg", 1)}`);
    const lib = await screen.findByRole("region", { name: "src/lib.rs" });
    expect(within(lib).getByText(/src\/old.rs → src\/lib.rs/)).toBeInTheDocument();
    expect(within(lib).getByText("+new line", { exact: false })).toBeInTheDocument();
    expect(within(lib).getByText("Why this?")).toBeInTheDocument();
    expect(within(lib).queryByText("General note")).not.toBeInTheDocument();
    expect(screen.getByText("Binary file not shown.")).toBeInTheDocument();
    expect(screen.getByText(/over 256 KiB/)).toBeInTheDocument();
  });

  it("sends a line comment with its revision, path and line", async () => {
    signIn();
    const updated = change(1, {
      comments: [
        {
          at: "2026-01-01T10:30:00Z",
          author: "usr_1",
          body: "Rename this",
          path: "src/lib.rs",
          line: 2,
          revision: 1,
        },
      ],
    });
    const { calls } = serveChange(1, change(1), {
      [`POST ${changePath(1)}/comments`]: updated,
    });
    renderApp(`/changes/${id("chg", 1)}`);
    const lib = await screen.findByRole("region", { name: "src/lib.rs" });
    await userEvent.click(await within(lib).findByRole("button", { name: "Comment on this line" }));
    const form = within(lib).getByRole("form", { name: "Comment on src/lib.rs line 2" });
    await userEvent.type(within(form).getByRole("textbox"), "Rename this");
    expect(within(form).getByText(/--revision 1 --path src\/lib.rs --line 2/)).toBeInTheDocument();
    await userEvent.click(within(form).getByRole("button", { name: "Comment" }));
    await waitFor(() => expect(calls.some((c) => c.path.endsWith("/comments"))).toBe(true));
    expect(calls.find((c) => c.path.endsWith("/comments"))?.body).toEqual({
      body: "Rename this",
      revision: 1,
      path: "src/lib.rs",
      line: 2,
    });
    expect(await within(lib).findByText("Rename this", { selector: "p" })).toBeInTheDocument();
    expect(within(lib).queryByRole("form")).not.toBeInTheDocument();
  });

  it("reads another revision's diff and comments on it", async () => {
    signIn();
    const two = change(1, {
      revisions: [
        {
          number: 1,
          base: "a".repeat(40),
          head: "b".repeat(40),
          created_at: "2026-01-01T10:00:00Z",
        },
        {
          number: 2,
          base: "a".repeat(40),
          head: "d".repeat(40),
          created_at: "2026-01-01T11:00:00Z",
        },
      ],
    });
    const { calls } = serveChange(1, two, {
      [`POST ${changePath(1)}/comments`]: two,
    });
    renderApp(`/changes/${id("chg", 1)}`);
    await screen.findByRole("region", { name: "src/lib.rs" });
    const selector = screen.getByRole("combobox", { name: "Revision" });
    expect(selector).toHaveValue("2");
    await userEvent.selectOptions(selector, "1");
    await waitFor(() =>
      expect(calls.filter((c) => c.path.endsWith("/diff")).map((c) => c.search)).toContain(
        "?revision=1",
      ),
    );
    await userEvent.click(screen.getByRole("tab", { name: /^Comments/ }));
    await userEvent.type(screen.getByLabelText("Comment on the change text"), "Looks fine");
    await userEvent.click(screen.getByRole("button", { name: "Comment" }));
    await waitFor(() => expect(calls.some((c) => c.path.endsWith("/comments"))).toBe(true));
    expect(calls.find((c) => c.path.endsWith("/comments"))?.body).toEqual({
      body: "Looks fine",
      revision: 1,
    });
  });

  it("opens the first failed check at the first error of its log", async () => {
    signIn();
    const logs = new StubEvents();
    const failed = run(1, {
      phase: "failed",
      checks: [
        { name: "lint", status: "passed", job: id("job", 101) },
        { name: "test", status: "failed", job: id("job", 102), exit_code: 101 },
        { name: "doc", status: "failed", job: id("job", 103), exit_code: 1 },
      ],
    });
    serveChange(1, change(1), {
      [`GET ${changePath(1)}/runs`]: [failed],
      [`GET /v1/jobs/${id("job", 102)}/logs`]: (request: Request) =>
        logs.open(request.headers.get("Last-Event-ID")),
      [`GET /v1/jobs/${id("job", 102)}`]: job(102, { phase: "finished", exit_code: 101 }),
    });
    renderApp(`/changes/${id("chg", 1)}`);
    await userEvent.click(await screen.findByRole("tab", { name: "Checks" }));
    const list = await screen.findByRole("list", { name: "Checks" });
    expect(within(list).getByRole("button", { name: /test/ })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await waitFor(() => expect(logs.connections).toHaveLength(1));
    logs.raw(
      `id: 1\nevent: stdout\ndata: running 3 tests\ndata: error[E0432]: unresolved import\ndata: more\ndata: \n\n`,
    );
    logs.raw(
      `id: 2\nevent: end\ndata: ${JSON.stringify(job(102, { phase: "finished", exit_code: 101 }))}\n\n`,
    );
    await screen.findByText("more");
    await waitFor(() =>
      expect(document.querySelector("[data-highlighted]")).toHaveTextContent("error[E0432]"),
    );
  });

  it("says when no checks ran for the revision", async () => {
    signIn();
    serveChange(1);
    renderApp(`/changes/${id("chg", 1)}`);
    await userEvent.click(await screen.findByRole("tab", { name: "Checks" }));
    expect(await screen.findByText("No checks have run for revision 1.")).toBeInTheDocument();
  });

  it("lists the comments and the timeline", async () => {
    signIn();
    serveChange(
      1,
      change(1, {
        comments: [
          { at: "2026-01-01T10:30:00Z", author: "usr_1", body: "First!", revision: 1 },
          {
            at: "2026-01-01T10:40:00Z",
            author: "usr_1",
            body: "On a line",
            path: "src/lib.rs",
            line: 9,
            revision: 1,
          },
        ],
        approvals: [{ at: "2026-01-01T11:00:00Z", by: "usr_2", revision: 1 }],
        phase: "merged",
        merged_commit: "c".repeat(40),
      }),
      { [`GET ${changePath(1)}/runs`]: [run(1, { started_at: "2026-01-01T10:10:00Z" })] },
    );
    renderApp(`/changes/${id("chg", 1)}`);
    await userEvent.click(await screen.findByRole("tab", { name: /^Comments/ }));
    expect(screen.getByText("First!")).toBeInTheDocument();
    expect(screen.getByText("src/lib.rs:9")).toBeInTheDocument();
    expect(screen.queryByRole("form")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("tab", { name: "Timeline" }));
    const timeline = await screen.findByRole("list", { name: "Timeline" });
    await within(timeline).findByText("Checks of revision 1 started");
    const items = within(timeline)
      .getAllByRole("listitem")
      .map((item) => item.textContent ?? "");
    expect(items[0]).toContain("Opened as revision 1");
    expect(items[1]).toContain("Checks of revision 1 started");
    expect(items[2]).toContain("usr_1 commented on revision 1");
    expect(items[4]).toContain("usr_2 approved revision 1");
    expect(items[5]).toContain("Merged as");
  });
});
