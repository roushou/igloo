import createClient, { type Middleware } from "openapi-fetch";
import type { components, paths } from "./schema.gen";
import { TokenStore } from "./token";

type Schemas = components["schemas"];

export type Repo = Schemas["RepoResource"];
export type Task = Schemas["TaskResource"];
export type Transcript = Schemas["TranscriptResource"];
export type TranscriptEntry = Schemas["TranscriptEntry"];
export type Change = Schemas["ChangeResource"];
export type ChangeDiff = Schemas["DiffResource"];
export type FileDiff = Schemas["FileDiffResource"];
export type Run = Schemas["RunResource"];
export type Check = Schemas["CheckResource"];
export type Job = Schemas["JobResource"];
export type WarmSnapshot = Schemas["WarmSnapshotResource"];
export type Sandbox = Schemas["SandboxResource"];
export type Worker = Schemas["WorkerResource"];
export type Storage = Schemas["StorageResource"];
export type Comment = Schemas["CommentResource"];
export type Problem = Schemas["Problem"];
export type TaskPhase = Schemas["TaskPhase"];
export type ChangePhase = Schemas["ChangePhase"];
export type TerminalClientFrame = Schemas["TerminalClientFrame"];
export type TerminalServerFrame = Schemas["TerminalServerFrame"];
export type TerminalEndReason = Schemas["TerminalEndReason"];

/** What a terminal runs and the screen it starts with. */
export type TerminalOptions = { command?: string[]; cols: number; rows: number };

/** The WebSocket subprotocol of a terminal, and the prefix of the one that carries the token. */
export const TERMINAL_PROTOCOL = "igloo.terminal.v1";
const TERMINAL_BEARER_PREFIX = "igloo.bearer.";

/** How a token fared when checked against the server. */
export type TokenCheck = "accepted" | "rejected" | "unreachable";

/** A request the server refused. Its problem document names the rule that was not met. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly problem: Problem | null,
  ) {
    super(problem?.detail ?? problem?.title ?? `request failed with ${status}`);
  }

  /** The stable code of the problem, such as `change.approval_required`; `null` without one. */
  get code(): string | null {
    return this.problem?.code ?? null;
  }
}

/** What an `openapi-fetch` call resolves to. */
type Reply<T> = { data?: T; error?: unknown; response: Response };

function problemOf(error: unknown): Problem | null {
  return error && typeof error === "object" && "title" in error ? (error as Problem) : null;
}

/** The data of a reply; throws the `ApiError` of a refusal. */
function unwrap<T>(reply: Reply<T>): T {
  if (reply.error !== undefined || reply.data === undefined) {
    throw new ApiError(reply.response.status, problemOf(reply.error));
  }
  return reply.data;
}

/** Throws the `ApiError` of a refusal of a request that returns nothing. */
function expectOk(reply: { error?: unknown; response: Response }): void {
  if (reply.error !== undefined) throw new ApiError(reply.response.status, problemOf(reply.error));
}

/**
 * The only module that talks to the server: every request goes through it, carries the stored
 * bearer token, and a 401 on any of them clears the token and calls the unauthorized handler.
 * It touches the browser only when a request is made, so the build can import it.
 */
export class ApiClient {
  readonly tokens: TokenStore;
  private onUnauthorized: () => void = () => {};
  private client: ReturnType<typeof createClient<paths>> | null = null;

  constructor(tokens = new TokenStore()) {
    this.tokens = tokens;
  }

  /** Sets what happens after a 401, such as returning to the sign-in page. */
  handleUnauthorized(handler: () => void): void {
    this.onUnauthorized = handler;
  }

  /** The repositories, oldest first. */
  async repos(): Promise<Repo[]> {
    const { data, error } = await this.http.GET("/v1/repos");
    if (error || !data) {
      throw new Error("could not list repositories");
    }
    return data;
  }

  /** A repository's tasks, newest first, only those in `phases` when any are given. */
  async tasks(repo: string, phases: TaskPhase[] = []): Promise<Task[]> {
    return unwrap(
      await this.http.GET("/v1/repos/{id}/tasks", {
        params: { path: { id: repo }, query: { phase: phases, order: "newest" } },
      }),
    );
  }

  async task(id: string): Promise<Task> {
    return unwrap(await this.http.GET("/v1/tasks/{id}", { params: { path: { id } } }));
  }

  async createTask(repo: string, goal: string, tool?: string): Promise<Task> {
    return unwrap(
      await this.http.POST("/v1/repos/{id}/tasks", {
        params: { path: { id: repo } },
        body: { goal, tool: tool || null },
      }),
    );
  }

  async cancelTask(id: string): Promise<Task> {
    return unwrap(await this.http.POST("/v1/tasks/{id}/cancel", { params: { path: { id } } }));
  }

  /** A task's transcript from position `after` on. */
  async transcript(id: string, after: number): Promise<Transcript> {
    return unwrap(
      await this.http.GET("/v1/tasks/{id}/transcript", {
        params: { path: { id }, query: { after } },
      }),
    );
  }

  /** A repository's changes, newest first, only those in `phases` when any are given. */
  async changes(repo: string, phases: ChangePhase[] = []): Promise<Change[]> {
    return unwrap(
      await this.http.GET("/v1/repos/{id}/changes", {
        params: { path: { id: repo }, query: { phase: phases, order: "newest" } },
      }),
    );
  }

  async change(id: string): Promise<Change> {
    return unwrap(await this.http.GET("/v1/changes/{id}", { params: { path: { id } } }));
  }

  /** The files of a change's revision, the latest when `revision` is omitted. */
  async diff(id: string, revision?: number): Promise<ChangeDiff> {
    return unwrap(
      await this.http.GET("/v1/changes/{id}/diff", {
        params: { path: { id }, query: { revision } },
      }),
    );
  }

  /** The runs of a change's revisions, with their checks. */
  async changeRuns(id: string): Promise<Run[]> {
    return unwrap(await this.http.GET("/v1/changes/{id}/runs", { params: { path: { id } } }));
  }

  /** Comments on a revision of the change, or on a line of a file when `path` and `line` are set. */
  async comment(
    id: string,
    body: { body: string; revision?: number; path?: string; line?: number },
  ): Promise<Change> {
    return unwrap(
      await this.http.POST("/v1/changes/{id}/comments", { params: { path: { id } }, body }),
    );
  }

  async requestChanges(id: string): Promise<Change> {
    return unwrap(
      await this.http.POST("/v1/changes/{id}/request-changes", { params: { path: { id } } }),
    );
  }

  /** Approves the latest revision, which must still be `revision` when that is given. */
  async approve(id: string, revision?: number): Promise<Change> {
    return unwrap(
      await this.http.POST("/v1/changes/{id}/approve", {
        params: { path: { id } },
        body: { revision },
      }),
    );
  }

  async merge(id: string): Promise<Change> {
    return unwrap(await this.http.POST("/v1/changes/{id}/merge", { params: { path: { id } } }));
  }

  async close(id: string): Promise<Change> {
    return unwrap(await this.http.POST("/v1/changes/{id}/close", { params: { path: { id } } }));
  }

  /** Records the change's branch head as its next revision. */
  async revise(id: string): Promise<Change> {
    return unwrap(await this.http.POST("/v1/changes/{id}/revisions", { params: { path: { id } } }));
  }

  /** A repository's runs, newest first. */
  async runs(repo: string, limit = 50): Promise<Run[]> {
    const list = unwrap(
      await this.http.GET("/v1/repos/{id}/runs", {
        params: { path: { id: repo }, query: { limit } },
      }),
    );
    return list.items;
  }

  async run(id: string): Promise<Run> {
    return unwrap(await this.http.GET("/v1/runs/{id}", { params: { path: { id } } }));
  }

  /** The warm and agent snapshots a repository recorded. */
  async repoSnapshots(repo: string): Promise<WarmSnapshot[]> {
    return unwrap(
      await this.http.GET("/v1/repos/{id}/snapshots", { params: { path: { id: repo } } }),
    );
  }

  async job(id: string): Promise<Job> {
    return unwrap(await this.http.GET("/v1/jobs/{id}", { params: { path: { id } } }));
  }

  async workers(): Promise<Worker[]> {
    return unwrap(await this.http.GET("/v1/workers"));
  }

  /** What the blob store holds and what its latest sweep reclaimed. */
  async storage(): Promise<Storage> {
    return unwrap(await this.http.GET("/v1/storage"));
  }

  /** The sandboxes of every repository. */
  async sandboxes(): Promise<Sandbox[]> {
    const list = unwrap(
      await this.http.GET("/v1/sandboxes", { params: { query: { limit: 200 } } }),
    );
    return list.items;
  }

  /** The names of a repository's secrets. The server never returns values. */
  async secrets(repo: string): Promise<string[]> {
    const list = unwrap(
      await this.http.GET("/v1/repos/{id}/secrets", { params: { path: { id: repo } } }),
    );
    return list.names;
  }

  async setSecret(repo: string, name: string, value: string): Promise<void> {
    expectOk(
      await this.http.PUT("/v1/repos/{id}/secrets/{name}", {
        params: { path: { id: repo, name } },
        body: value,
        bodySerializer: (body) => body,
        headers: { "Content-Type": "text/plain" },
      }),
    );
  }

  async deleteSecret(repo: string, name: string): Promise<void> {
    expectOk(
      await this.http.DELETE("/v1/repos/{id}/secrets/{name}", {
        params: { path: { id: repo, name } },
      }),
    );
  }

  /**
   * Opens the event stream, resuming after event `lastId` when given. A 401 is handled like any
   * other request; any other failure rejects.
   */
  events(lastId: string | null, signal: AbortSignal): Promise<ReadableStream<Uint8Array>> {
    return this.stream("/v1/events", lastId, signal);
  }

  /** Opens a job's output stream, resuming after event `lastId` when given. */
  jobLogs(id: string, lastId: string | null, signal: AbortSignal) {
    return this.stream(`/v1/jobs/${encodeURIComponent(id)}/logs`, lastId, signal);
  }

  private async stream(
    path: string,
    lastId: string | null,
    signal: AbortSignal,
  ): Promise<ReadableStream<Uint8Array>> {
    const headers = new Headers({ Accept: "text/event-stream" });
    const token = this.tokens.get();
    if (token) headers.set("Authorization", `Bearer ${token}`);
    if (lastId !== null) headers.set("Last-Event-ID", lastId);
    const response = await fetch(new URL(path, window.location.origin), { headers, signal });
    if (response.status === 401) {
      this.tokens.clear();
      this.onUnauthorized();
    }
    if (!response.ok || !response.body) {
      throw new Error(`event stream answered ${response.status}`);
    }
    return response.body;
  }

  /**
   * Opens the terminal WebSocket of a running sandbox. The socket speaks `TERMINAL_PROTOCOL`; the
   * bearer token travels as a second subprotocol because a browser cannot set headers on a
   * WebSocket. Throws when the token is not valid in a subprotocol. A refusal arrives as the
   * socket closing before it opened.
   */
  terminalSocket(sandbox: string, options: TerminalOptions): WebSocket {
    const url = new URL(
      `/v1/sandboxes/${encodeURIComponent(sandbox)}/terminal`,
      window.location.origin,
    );
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    for (const part of options.command ?? []) url.searchParams.append("command", part);
    url.searchParams.set("cols", String(options.cols));
    url.searchParams.set("rows", String(options.rows));
    const protocols: string[] = [TERMINAL_PROTOCOL];
    const token = this.tokens.get();
    if (token) protocols.push(`${TERMINAL_BEARER_PREFIX}${token}`);
    const socket = new WebSocket(url, protocols);
    socket.binaryType = "arraybuffer";
    return socket;
  }

  /** Checks `token` with `GET /v1/repos` without storing it or triggering the 401 handler. */
  async check(token: string): Promise<TokenCheck> {
    try {
      const response = await fetch(new URL("/v1/repos", window.location.origin), {
        headers: { Authorization: `Bearer ${token}` },
      });
      if (response.ok) return "accepted";
      return response.status === 401 || response.status === 403 ? "rejected" : "unreachable";
    } catch {
      return "unreachable";
    }
  }

  private get http() {
    if (!this.client) {
      const auth: Middleware = {
        onRequest: ({ request }) => {
          const token = this.tokens.get();
          if (token && !request.headers.has("Authorization")) {
            request.headers.set("Authorization", `Bearer ${token}`);
          }
          return request;
        },
        onResponse: ({ response }) => {
          if (response.status === 401) {
            this.tokens.clear();
            this.onUnauthorized();
          }
          return response;
        },
      };
      this.client = createClient<paths>({
        baseUrl: window.location.origin,
        fetch: (request) => globalThis.fetch(request),
      });
      this.client.use(auth);
    }
    return this.client;
  }
}

export const api = new ApiClient();
