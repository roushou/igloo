# The web console

The console is a web page served by `igloo-control` at `/` (ADR 0012). It does everything the
`igloo` CLI does for tasks, changes and secrets, and shows what the CLI cannot: what waits on you,
what runs, and the diff, transcript and logs of both.

## Opening it

The server listens on localhost on the host. Forward the port and open it:

```sh
mise run tunnel                  # forwards the API to http://127.0.0.1:7000
mise run token                   # prints the API token
```

Open `http://127.0.0.1:7000/` and paste the token on the sign-in page. The token stays in the
browser (`localStorage`) until you sign out or the server answers 401. This is acceptable only while
the server is reachable on localhost or a tailnet.

The console is built into `IGLOO_WEB_DIR` by `deploy/vps.sh update` (`docs/deploy.md`). Without
it, `/` is 404 and the API still works.

## The rail

The left rail holds the repository switcher, the pages, the connection state and sign out.
`⌘K` (or `Ctrl+K`) opens a resource by id: paste a `task_`, `chg_`, `run_`, `job_` or `sbx_` id, or
a `repo_` id to switch repository.

The console follows one event stream. When the connection to the server is lost the rail reads
_Reconnecting…_, then _Offline_; when it returns, the console resumes from the last event it saw and
refetches what changed. Nothing needs a reload.

## What the colors mean

Every page uses the same six states, one pill each:

| State     | Meaning                                                                    |
| --------- | -------------------------------------------------------------------------- |
| Needs you | Orange. Waits on you: a task to review, a change to approve or merge.      |
| Running   | Blue. Work in progress.                                                    |
| Passed    | Green. A passed run or check, an exit 0, a merged change, a finished task. |
| Failed    | Red. The code is at fault: a failed check, an exit other than 0.           |
| Errored   | Amber. Igloo or the machine is at fault, never the code.                   |
| Closed    | Grey. Cancelled, closed or stopped.                                        |

Orange never means an error. Logs, tool output and diffs are dark in both themes. Ids are shortened
(`task_01j9…v2w3`); click one to copy the whole id.

## Pages

**Now** (`/`) puts what needs you first, then what runs, then history. _Needs you_ lists tasks
awaiting review and open changes whose checks passed or that need an approval, each with what it
waits on. _Running_ lists working tasks and runs with their step and elapsed time. _Recent runs_
shows how runs ended, with the reason of an errored one. _Recent activity_ lists events as they
arrive.

**Tasks** (`/tasks`) lists the repository's tasks grouped by state. _Create task_ takes a goal and
optionally a tool of `.igloo/agents.toml`. A task (`/tasks/:id`) shows its goal, state, sandbox and
change, and a transcript that grows while the agent works: each tool call is one line that expands
to its input and output, and edits show as removed and added lines. _Cancel task_ stops it.
_Watch the sandbox_ opens a read-only terminal on it. _Take over_ makes the terminal writable for you
and starts no further turn once the running one ends; _Hand back_ tells the agent what you changed
and resumes the task.

**Changes** (`/changes`) lists changes grouped by state. A change (`/changes/:id`) shows the merge
checklist (checks, approval, fast-forward) above four tabs:

- _Diff_: the files of the selected revision. Hover a line and use **+** to comment on it; the
  comment is sent with the revision, file and line. Comments on added and unchanged lines only.
- _Checks_: the checks of the revision's latest run. The first failed check is selected and its
  log opens at its first error.
- _Comments_: every comment, and a box for a comment on the change.
- _Timeline_: revisions, runs, comments and approvals in order.

The revision selector chooses which revision the Diff, Checks and new comments are about. The
actions are _Record revision_, _Request changes_, _Approve_, _Merge_ and _Close_. A disabled action
says which rule is unmet, for example _A human must approve it: it touches proto/a.proto_.

Every action has a terminal button beside it that shows the equivalent `igloo` command.

**Run** (`/runs/:id`) lists a run's checks with a link to each log; **Job log** (`/jobs/:id`) shows
a job's whole output, following it while it runs. Search the log, step through matches, or jump to
the first line that mentions `error`. _Follow_ stops when you scroll up.

**System** (`/system`) shows each worker's connection, schedulability, allocated CPU and memory,
disk use and layer cache against its limit, and when it last reported. Below are the sandboxes
with the task or run that owns each; `/system?sandbox=<id>` marks one.

**Repository** (`/settings`) shows the forge location, default branch and token secret name, and
the secret names. _Set secret_ takes a name and a masked value; the value is sent once and never
shown or returned. Setting an existing name replaces it. _Delete_ asks for confirmation.

## Not in the console

Editing `.igloo/` files, outcome and metric pages, and any authentication beyond the bearer token
(ADR 0012; `docs/specs/phase-5.md`, out of scope).
