// Generated from schemas/openapi.json by `bun run gen:api`. Do not edit.
export interface paths {
    "/v1/blobs/{digest}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Downloads a blob. Authenticated by the bearer token or a presigned download URL. */
        get: operations["getBlob"];
        /**
         * Uploads bytes under their blake3 digest. Idempotent; rejected if the bytes do not match.
         *     Authenticated by the bearer token or a presigned upload URL.
         */
        put: operations["putBlob"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/changes/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Gets a change. */
        get: operations["getChange"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/changes/{id}/approve": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Approves a change's latest revision as the caller, who must be a human. */
        post: operations["approveChange"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/changes/{id}/close": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Closes a change without merging. */
        post: operations["closeChange"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/changes/{id}/comments": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Comments on a revision of a change, the latest by default, optionally on a line of a file. */
        post: operations["commentOnChange"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/changes/{id}/diff": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * What a revision of a change changes: the files that differ between the revision's base and
         *     head in the repository's mirror, each with its unified patch as git prints it. A patch over
         *     256 KiB is left out and its file marked truncated.
         */
        get: operations["getChangeDiff"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/changes/{id}/merge": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * Merges a change: its latest revision's checks must have passed, it must be based on the
         *     target branch's head, and changes to protected paths need a human approval. Igloo pushes
         *     the revision to the target branch as one squashed commit naming the change.
         */
        post: operations["mergeChange"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/changes/{id}/request-changes": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * Asks for another revision, carrying the comments made since the previous request. A change
         *     made by a task sends them to its agent as its next turn.
         */
        post: operations["requestChanges"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/changes/{id}/revisions": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * Records the source branch's current head as the next revision; an unchanged head records
         *     nothing.
         */
        post: operations["reviseChange"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/changes/{id}/runs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Lists the runs of a change, one per revision, oldest first. */
        get: operations["listChangeRuns"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/events": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * Streams a notice per committed event as server-sent events: `id` is the event's sequence,
         *     `event` its kind, `data` an `EventNotice`. `repo` keeps only events of that repository's
         *     resources. `after`, or else `Last-Event-ID`, resumes after that sequence; without either the stream
         *     starts at the log's current head and sends only events committed after it opened, and
         *     `after=0` replays everything. Events are sent in sequence order, each once. Comments keep the connection alive every 15 seconds.
         */
        get: operations["stream"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/jobs/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Gets a job. */
        get: operations["getJob"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/jobs/{id}/logs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * Streams a job's output as server-sent events: `stdout` and `stderr` events whose id resumes
         *     the stream through `Last-Event-ID`, then one `end` event with the finished job.
         */
        get: operations["getJobLogs"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/repos": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Lists repositories, oldest first. */
        get: operations["listRepos"];
        put?: never;
        /** Registers a repository. */
        post: operations["registerRepo"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/repos/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Gets a repository. */
        get: operations["getRepo"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/repos/{id}/changes": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * Lists a repository's changes, oldest first unless `order=newest`, optionally only those at
         *     the given phases. Open changes carry their merge readiness.
         */
        get: operations["listRepoChanges"];
        put?: never;
        /** Opens a change proposing a branch pushed to the forge; its head is the first revision. */
        post: operations["openChange"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/repos/{id}/runs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Lists a repository's runs, newest first, a page at a time. */
        get: operations["listRepoRuns"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/repos/{id}/secrets": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Lists the names of a repository's secrets; values are never returned. */
        get: operations["listRepoSecrets"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/repos/{id}/secrets/{name}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        /** Sets or replaces a secret. The body is its value, as text. */
        put: operations["setRepoSecret"];
        post?: never;
        /** Deletes a secret. */
        delete: operations["deleteRepoSecret"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/repos/{id}/snapshots": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * Lists the snapshots a repository recorded under keys, warm and agent, ordered by key, with
         *     when each was last used. Sizes are not reported yet.
         */
        get: operations["listRepoSnapshots"];
        put?: never;
        /**
         * Snapshots a repository at a commit: the checkout under `/workspace`, with a shallow `.git`,
         *     over an optional base snapshot. A branch is fetched from the forge first.
         */
        post: operations["snapshotRepo"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/repos/{id}/tasks": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * Lists a repository's tasks, oldest first unless `order=newest`, optionally only those at the
         *     given phases.
         */
        get: operations["listRepoTasks"];
        put?: never;
        /** Creates a task: the agent starts from the head of the repository's default branch. */
        post: operations["createTask"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/repos/{id}/workspaces": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * Opens a workspace on a branch of a repository for the caller. It starts at once: its sandbox
         *     is forked from the repository's warm snapshot at the branch head.
         */
        post: operations["createWorkspace"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/runs/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Gets a run. */
        get: operations["getRun"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/sandboxes": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Lists sandboxes, ordered by id, optionally filtered by labels (`label=key=value`, repeatable). */
        get: operations["listSandboxes"];
        put?: never;
        /** Creates a sandbox from a registered snapshot. */
        post: operations["createSandbox"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/sandboxes/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Gets a sandbox. */
        get: operations["getSandbox"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/sandboxes/{id}/exec": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Runs a process in a sandbox as a job. */
        post: operations["execInSandbox"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/sandboxes/{id}/snapshot": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * Seals a running sandbox into a snapshot. The seal completes in the background; poll it with
         *     `GET /v1/seals/{id}`.
         */
        post: operations["createSeal"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/sandboxes/{id}/stop": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Stops a sandbox. Idempotent. */
        post: operations["stopSandbox"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/sandboxes/{id}/terminal": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * Opens an interactive terminal in a running sandbox: upgrades to a WebSocket speaking the
         *     subprotocol `igloo.terminal.v1`.
         * @description Binary frames carry the terminal's bytes in both directions. Text frames carry JSON control
         *     messages: the client sends `TerminalClientFrame` (resize); the server sends one
         *     `TerminalServerFrame` (exit) and closes the socket when the process ends. Closing the socket
         *     kills the process. Authenticated like the rest of the API; a browser, which cannot set headers
         *     on a WebSocket, offers the subprotocol `igloo.bearer.<token>` as well.
         */
        get: operations["openTerminal"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/seals/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Gets a seal. */
        get: operations["getSeal"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/seals/{id}/layer": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        /**
         * Uploads the layer that completes a seal, as a tar archive. Authenticated only by the
         *     presigned URL handed to the sandbox's worker.
         */
        put: operations["uploadSealLayer"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/snapshots": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Registers a snapshot of uploaded layers, over a base snapshot's layers if one is named. */
        post: operations["createSnapshot"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/snapshots/import": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** Pulls a public container image from its registry and registers its layers as a snapshot. */
        post: operations["importImage"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/snapshots/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Gets a snapshot. */
        get: operations["getSnapshot"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/storage": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Reports what the blob store holds and what its latest sweep reclaimed. */
        get: operations["getStorage"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/tasks/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Gets a task. */
        get: operations["getTask"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/tasks/{id}/cancel": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * Cancels a task and stops its sandbox. Cancelling an ended task changes nothing, except that
         *     a task that failed collecting its commits stops the sandbox it kept for recovery.
         */
        post: operations["cancelTask"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/tasks/{id}/transcript": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * A task's transcript from a position on: what its tool said and did, turn by turn, read from
         *     its output with secrets masked. Poll with `after` set to the last response's `next`.
         */
        get: operations["getTaskTranscript"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/workers": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Lists the workers, ordered by id, with what their sandboxes hold of them. */
        get: operations["listWorkers"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/workspaces": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Lists the caller's workspaces, oldest first. */
        get: operations["listWorkspaces"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/workspaces/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Gets a workspace. */
        get: operations["getWorkspace"];
        put?: never;
        post?: never;
        /** Deletes a workspace: its sandbox is stopped without sealing and its changes are dropped. */
        delete: operations["deleteWorkspace"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/workspaces/{id}/start": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * Starts a workspace: a sandbox resumes from what its last stop sealed. Starting a running
         *     workspace changes nothing; a workspace that is stopping starts again once it has stopped.
         */
        post: operations["startWorkspace"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/workspaces/{id}/stop": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * Stops a workspace: its changes are sealed, then its sandbox stops. Stopping a stopped
         *     workspace changes nothing.
         */
        post: operations["stopWorkspace"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
}
export type webhooks = Record<string, never>;
export interface components {
    schemas: {
        /** @description The approval a change needs. */
        ApprovalNeed: {
            /** @description The protected paths the change touches; empty when `state` is `not_required`. */
            protected_paths?: string[];
            /** @description Whether it is needed and given. */
            state: components["schemas"]["ApprovalState"];
        };
        /** @description A human's approval of a revision. */
        ApprovalResource: {
            /**
             * Format: date-time
             * @description When.
             */
            at: string;
            /** @description Who approved it (`usr_...`). */
            by: string;
            /**
             * Format: int32
             * @description The revision approved.
             */
            revision: number;
        };
        /**
         * @description Whether a human must approve a change before it merges.
         * @enum {string}
         */
        ApprovalState: "not_required" | "required" | "given";
        /** @description Approves a change's latest revision. */
        ApproveRequest: {
            /**
             * Format: int32
             * @description The revision approved; it must be the latest. The latest when omitted.
             */
            revision?: number | null;
        };
        /**
         * @description Where a change is.
         * @enum {string}
         */
        ChangePhase: "open" | "merged" | "closed";
        /** @description A change. */
        ChangeResource: {
            /** @description Human approvals, oldest first. */
            approvals: components["schemas"]["ApprovalResource"][];
            /**
             * Format: date-time
             * @description When it was closed; present only once closed.
             */
            closed_at?: string | null;
            /** @description Review comments, oldest first. */
            comments?: components["schemas"]["CommentResource"][];
            /** @description Its id (`chg_...`). */
            id: string;
            /**
             * Format: date-time
             * @description When it was merged; present only once merged.
             */
            merged_at?: string | null;
            /** @description The target branch's head after the merge, once merged. */
            merged_commit?: string | null;
            /** @description Where it is. */
            phase: components["schemas"]["ChangePhase"];
            readiness?: components["schemas"]["MergeReadiness"] | null;
            /** @description The repository. */
            repo: string;
            /** @description Every revision, oldest first. */
            revisions: components["schemas"]["RevisionResource"][];
            /** @description The branch proposed. */
            source_branch: string;
            /** @description The branch it merges into. */
            target_branch: string;
            /** @description What it does. */
            title: string;
        };
        /** @description One check of a run. */
        CheckResource: {
            /**
             * Format: date-time
             * @description When its job ended; absent until it has.
             */
            ended_at?: string | null;
            /**
             * Format: int32
             * @description The exit code, when it failed.
             */
            exit_code?: number | null;
            /** @description The job running it, whose logs are its output. */
            job?: string | null;
            /** @description Its name in the pipeline. */
            name: string;
            /** @description Why it did not complete. */
            reason?: string | null;
            /**
             * Format: date-time
             * @description When its job's process started; absent until it has.
             */
            started_at?: string | null;
            /** @description Where it is. */
            status: components["schemas"]["CheckStatus"];
        };
        /**
         * @description Where a check is.
         * @enum {string}
         */
        CheckStatus: "pending" | "started" | "passed" | "failed" | "errored";
        /**
         * @description Where the checks of a change's latest revision are.
         * @enum {string}
         */
        ChecksState: "passed" | "failed" | "running" | "missing";
        /** @description Comments on a change. */
        CommentRequest: {
            /** @description What it says; 1 to 10 000 characters. */
            body: string;
            /**
             * Format: int32
             * @description The line of `path` it is about, from 1.
             */
            line?: number | null;
            /** @description The file it is about, relative to the repository root. */
            path?: string | null;
            /**
             * Format: int32
             * @description The revision; the latest when omitted.
             */
            revision?: number | null;
        };
        /** @description A reviewer's comment on a revision. */
        CommentResource: {
            /**
             * Format: date-time
             * @description When.
             */
            at: string;
            /** @description Who wrote it: a user (`usr_...`), an agent (`agt_...`) or `system`. */
            author: string;
            /** @description What it says. */
            body: string;
            /**
             * Format: int32
             * @description The line of `path` it is about, from 1.
             */
            line?: number | null;
            /** @description The file it is about. */
            path?: string | null;
            /**
             * Format: int32
             * @description The revision commented on.
             */
            revision: number;
        };
        /** @description Creates a sandbox from a snapshot. */
        CreateSandboxRequest: {
            /** @description Environment variables of every process. */
            env?: {
                [key: string]: string;
            };
            isolation?: components["schemas"]["Isolation"] | null;
            /** @description Metadata for grouping and filtering. */
            labels?: {
                [key: string]: string;
            };
            limits?: components["schemas"]["Limits"] | null;
            network?: components["schemas"]["NetworkPolicy"] | null;
            /** @description The repository the sandbox works on (`repo_...`); its secrets become available to jobs. */
            repo?: string | null;
            /** @description The snapshot to start from. */
            snapshot: string;
        };
        /** @description Registers a snapshot from uploaded blobs, optionally on top of an existing snapshot. */
        CreateSnapshotRequest: {
            /** @description A snapshot whose layers go underneath, such as an imported image. */
            base?: string | null;
            /** @description Layers, lowest first. */
            layers: components["schemas"]["Layer"][];
        };
        /** @description Creates a task on a repository. */
        CreateTaskRequest: {
            /** @description What the agent is asked to do; not empty. Its first line titles the task's change. */
            goal: string;
            /** @description A tool of the repository's `.igloo/agents.toml`; its default tool when absent. */
            tool?: string | null;
        };
        /** @description Opens a workspace on a branch of a repository. */
        CreateWorkspaceRequest: {
            /** @description The branch to work on; it must exist. The repository's default branch when absent. */
            branch?: string | null;
        };
        /**
         * @description Whether the sandbox should run.
         * @enum {string}
         */
        DesiredState: "running" | "stopped";
        /** @description What one revision of a change changes, relative to where it forked from the target branch. */
        DiffResource: {
            /** @description The commit the diff starts from: the revision's base. */
            base: string;
            /** @description The files that differ, sorted by path. */
            files: components["schemas"]["FileDiffResource"][];
            /** @description The commit the diff ends at: the revision's head. */
            head: string;
            /**
             * Format: int32
             * @description The revision, from 1.
             */
            revision: number;
        };
        /**
         * @description What `GET /v1/events` sends as the `data` of each server-sent event: which resource changed
         *     and when, never the domain payload. Clients refetch the resource.
         */
        EventNotice: {
            /** @description The event kind, such as `igloo.run.passed`; also the SSE `event`. */
            kind: string;
            /** @description The repository the resource belongs to, when it has one. */
            repo?: string | null;
            /** @description The resource's id, such as `run_...`. */
            resource_id: string;
            /** @description The kind of resource the event belongs to, such as `run`. */
            resource_type: string;
            /**
             * Format: int64
             * @description The event's position in the global log; also the SSE `id`.
             */
            sequence: number;
            /**
             * Format: date-time
             * @description When the event was committed.
             */
            time: string;
        };
        /** @description Runs a process in a sandbox. */
        ExecRequest: {
            /** @description The program and its arguments. */
            argv: string[];
            /** @description Extra environment, layered over the sandbox's. */
            env?: {
                [key: string]: string;
            };
            /** @description Secrets of the sandbox's repository to expose as environment variables, by name. */
            secrets?: string[];
            /**
             * Format: int32
             * @description How long the process may run, 1 to 86400 seconds; one hour when omitted.
             */
            timeout_seconds?: number | null;
        };
        /** @description One invalid field of a request. */
        FieldError: {
            /** @description The field path, such as `"limits.millicpus"`. */
            field: string;
            /** @description Why it is invalid. */
            message: string;
        };
        /**
         * @description How a file differs in a change's diff.
         * @enum {string}
         */
        FileChangeStatus: "added" | "modified" | "deleted" | "renamed";
        /**
         * @description One file of a change's diff.
         *
         *     A file has a `patch` unless it is binary or `truncated`.
         */
        FileDiffResource: {
            /**
             * Format: int64
             * @description Lines added; 0 for a binary file.
             */
            additions: number;
            /** @description Whether the file is binary. */
            binary: boolean;
            /**
             * Format: int64
             * @description Lines removed; 0 for a binary file.
             */
            deletions: number;
            /**
             * @description The unified patch exactly as git prints it, from its `diff --git` header on. Absent for
             *     a binary file and when `truncated`.
             */
            patch?: string | null;
            /** @description Its path after the change; for a deleted file, the path it had. */
            path: string;
            /** @description The path a renamed file had before. */
            previous_path?: string | null;
            /** @description How it differs. */
            status: components["schemas"]["FileChangeStatus"];
            /** @description Whether the patch was left out for exceeding [`FileDiffResource::MAX_PATCH_BYTES`]. */
            truncated: boolean;
        };
        /** @description Imports a public OCI image as a snapshot. */
        ImportImageRequest: {
            /** @description An image reference, such as `docker.io/library/rust:1.99` or `rust:1.99`. */
            image: string;
            /** @description `os/architecture`, such as `linux/arm64`; the server's architecture when omitted. */
            platform?: string | null;
        };
        /**
         * @description How strongly a sandbox is separated from its host.
         * @enum {string}
         */
        Isolation: "any" | "container";
        /**
         * @description Where a job is in its lifecycle.
         * @enum {string}
         */
        JobPhase: "queued" | "leased" | "running" | "finished" | "failed" | "cancelled";
        /** @description A job. */
        JobResource: {
            /** @description The program and its arguments. */
            argv: string[];
            /**
             * Format: date-time
             * @description When it finished, failed or was cancelled; absent until it has.
             */
            ended_at?: string | null;
            /** @description Extra environment. */
            env: {
                [key: string]: string;
            };
            /**
             * Format: int32
             * @description The exit code, when finished.
             */
            exit_code?: number | null;
            /** @description Why it failed, when failed. */
            failure_reason?: string | null;
            /** @description Its id (`job_...`). */
            id: string;
            /** @description The phase. */
            phase: components["schemas"]["JobPhase"];
            /** @description The sandbox it runs in. */
            sandbox: string;
            /** @description The repository secrets it is given, by name. */
            secrets?: string[];
            /**
             * Format: date-time
             * @description When its process started; absent until it has.
             */
            started_at?: string | null;
            /**
             * Format: date-time
             * @description When it was submitted.
             */
            submitted_at: string;
            /**
             * Format: int64
             * @description How long the process may run.
             */
            timeout_seconds: number;
        };
        /** @description One layer of a snapshot. */
        Layer: {
            /** @description The uploaded blob's digest. */
            digest: string;
            /** @description How the blob is encoded; `tar` when omitted. */
            media_type?: components["schemas"]["MediaType"];
        };
        /** @description CPU and memory bounds. */
        Limits: {
            /**
             * Format: int32
             * @description Memory in MiB, 128 to 262144.
             */
            memory_mib: number;
            /**
             * Format: int32
             * @description CPU in thousandths of a core, 100 to 64000.
             */
            millicpus: number;
        };
        /**
         * @description The order of a list. Lists are oldest first unless asked otherwise.
         * @enum {string}
         */
        ListOrder: "oldest" | "newest";
        /**
         * @description How a layer blob is encoded.
         * @enum {string}
         */
        MediaType: "tar" | "tar+gzip";
        /**
         * @description What stands between an open change and its merge, by the rules merging applies. Merging
         *     stays authoritative: the target may move after this was computed.
         */
        MergeReadiness: {
            /** @description The approval of the latest revision. */
            approval: components["schemas"]["ApprovalNeed"];
            /** @description The checks of the latest revision. */
            checks: components["schemas"]["ChecksState"];
            /** @description Whether the latest revision's base is the target branch's head, so it merges as is. */
            fast_forward: boolean;
        };
        /**
         * @description Network access of a sandbox.
         * @enum {string}
         */
        NetworkPolicy: "deny_all" | "allow_all";
        /** @description Opens a change for a branch pushed to the forge. */
        OpenChangeRequest: {
            /** @description The branch proposed. */
            branch: string;
            /** @description What the change does, 1 to 200 characters. */
            title: string;
        };
        /**
         * @description An error response. `code` is stable and machine-readable; `title` and `detail` are for
         *     humans.
         */
        Problem: {
            /** @description The stable error code, such as `"sandbox.not_found"`. */
            code: string;
            /** @description What went wrong in this occurrence. */
            detail?: string | null;
            /** @description Every invalid field, for validation problems. */
            errors?: components["schemas"]["FieldError"][];
            /**
             * Format: int32
             * @description The HTTP status code.
             */
            status: number;
            /** @description A short summary of the problem type. */
            title: string;
            /** @description A URI identifying the problem type. */
            type: string;
        };
        /** @description Registers a repository. */
        RegisterRepoRequest: {
            /** @description The branch changes merge into; `main` when omitted. */
            default_branch?: string | null;
            /** @description `github.com/<owner>/<name>`, or an absolute path on the server for development. */
            location: string;
            /** @description The repository secret holding the forge token, such as `GITHUB_TOKEN`. */
            token_secret?: string | null;
        };
        /** @description A repository. */
        RepoResource: {
            /** @description The branch changes merge into. */
            default_branch: string;
            /** @description Its id (`repo_...`). */
            id: string;
            /** @description Where it is hosted. */
            location: string;
            /** @description The secret holding the forge token. */
            token_secret?: string | null;
        };
        /**
         * @description Makes a snapshot of a repository at a commit: its checkout under `/workspace`, with a shallow
         *     `.git`, over `base`. Names exactly one of `branch` (fetched first) and `commit`.
         */
        RepoSnapshotRequest: {
            /** @description The snapshot to check out over, such as an imported image. */
            base?: string | null;
            /** @description A branch on the forge. */
            branch?: string | null;
            /** @description A commit of the default branch's history. */
            commit?: string | null;
        };
        /** @description A repository snapshot. */
        RepoSnapshotResource: {
            /** @description The commit checked out. */
            commit: string;
            /** @description The snapshot. */
            snapshot: string;
        };
        /** @description One revision of a change. */
        RevisionResource: {
            /** @description Where it forked from the target branch. */
            base: string;
            /**
             * Format: date-time
             * @description When it was recorded.
             */
            created_at: string;
            /** @description The source branch's head. */
            head: string;
            /**
             * Format: int32
             * @description Its position, from 1.
             */
            number: number;
        };
        /** @description A page of runs, newest first. */
        RunList: {
            /** @description The runs, newest first. */
            items: components["schemas"]["RunResource"][];
            /** @description Pass as `cursor` to get the next page; absent on the last page. */
            next_cursor?: string | null;
        };
        /**
         * @description Where a run is.
         * @enum {string}
         */
        RunPhase: "preparing" | "warming" | "checking" | "passed" | "failed" | "errored";
        /** @description A run of a change's revision. */
        RunResource: {
            /** @description The change. */
            change: string;
            /** @description Its checks, in pipeline order; empty until prepared. */
            checks: components["schemas"]["CheckResource"][];
            /** @description The commit checked. */
            commit: string;
            /**
             * Format: date-time
             * @description When it ended, passed, failed or errored; absent while it runs.
             */
            ended_at?: string | null;
            /** @description Why it errored. */
            error?: string | null;
            /** @description Its id (`run_...`). */
            id: string;
            /** @description Where it is. */
            phase: components["schemas"]["RunPhase"];
            /**
             * Format: int32
             * @description The revision's number.
             */
            revision: number;
            /** @description The sandbox its checks run in; absent until they start. */
            sandbox?: string | null;
            /**
             * Format: date-time
             * @description When it started.
             */
            started_at: string;
            /** @description The job of the warm build this run did, if any; its logs are the build's output. */
            warm_job?: string | null;
        };
        /** @description A page of sandboxes. */
        SandboxList: {
            /** @description The sandboxes, ordered by id. */
            items: components["schemas"]["SandboxResource"][];
            /** @description Pass as `cursor` to get the next page; absent on the last page. */
            next_cursor?: string | null;
        };
        /**
         * @description Where a sandbox is in its lifecycle.
         * @enum {string}
         */
        SandboxPhase: "pending" | "scheduled" | "starting" | "running" | "stopping" | "stopped" | "failed";
        /** @description A sandbox: its desired spec and observed status. */
        SandboxResource: {
            /**
             * Format: date-time
             * @description When it was created.
             */
            created_at: string;
            /** @description Whether it should run. */
            desired: components["schemas"]["DesiredState"];
            /** @description Environment variables. */
            env: {
                [key: string]: string;
            };
            /** @description Why it failed, when `phase` is `failed`. */
            failure_reason?: string | null;
            /**
             * Format: int64
             * @description The spec generation; bumped by every spec change.
             */
            generation: number;
            /** @description Its id (`sbx_...`). */
            id: string;
            /** @description Separation from the host. */
            isolation: components["schemas"]["Isolation"];
            /** @description Labels. */
            labels: {
                [key: string]: string;
            };
            /** @description CPU and memory bounds. */
            limits: components["schemas"]["Limits"];
            /** @description Network access. */
            network: components["schemas"]["NetworkPolicy"];
            /**
             * Format: int64
             * @description The generation the worker last reported acting on.
             */
            observed_generation?: number | null;
            /** @description The observed phase. */
            phase: components["schemas"]["SandboxPhase"];
            /** @description The repository it works on. */
            repo?: string | null;
            /** @description The snapshot it starts from. */
            snapshot: string;
            /** @description The worker it is assigned to. */
            worker?: string | null;
        };
        /**
         * @description Where a seal is.
         * @enum {string}
         */
        SealPhase: "pending" | "sealed" | "failed";
        /** @description A seal of a sandbox into a snapshot. */
        SealResource: {
            /** @description Why it failed: `sandbox_ended` or `worker_error`. */
            failure_reason?: string | null;
            /** @description Its id (`seal_...`). */
            id: string;
            /** @description Where it is. */
            phase: components["schemas"]["SealPhase"];
            /** @description The sealed sandbox. */
            sandbox: string;
            /** @description The new snapshot, once sealed. */
            snapshot?: string | null;
        };
        /** @description The names of a repository's secrets. Values are never returned. */
        SecretList: {
            /** @description The names, sorted. */
            names: string[];
        };
        /** @description A snapshot. */
        SnapshotResource: {
            /** @description Its id: the digest of its manifest. */
            id: string;
            /** @description Layers, lowest first. */
            layers: components["schemas"]["Layer"][];
        };
        /** @description What the blob store holds and what its latest sweep reclaimed. */
        StorageResource: {
            /**
             * Format: int64
             * @description Blobs stored: layers and snapshot manifests.
             */
            blobs: number;
            /**
             * Format: int64
             * @description Their total size in bytes.
             */
            bytes: number;
            last_sweep?: components["schemas"]["StorageSweep"] | null;
            /** @description The repositories' recorded warm and agent snapshots, ordered by repository then key. */
            snapshots: components["schemas"]["StorageSnapshot"][];
        };
        /** @description A recorded warm or agent snapshot and its size. */
        StorageSnapshot: {
            /** @description The key it is recorded under. */
            key: string;
            /** @description The repository that recorded it (`repo_...`). */
            repo: string;
            /**
             * Format: int64
             * @description The sum of its layers' sizes in bytes, from file metadata; layers shared with other
             *     snapshots count in each.
             */
            size_bytes: number;
            /** @description The snapshot's id. */
            snapshot: string;
        };
        /** @description What one sweep kept and reclaimed. */
        StorageSweep: {
            /**
             * Format: date-time
             * @description When the sweep finished.
             */
            at: string;
            /**
             * Format: int64
             * @description Blobs left in the store.
             */
            kept_blobs: number;
            /**
             * Format: int64
             * @description Their size in bytes.
             */
            kept_bytes: number;
            /**
             * Format: int64
             * @description Blobs the sweep deleted.
             */
            reclaimed_blobs: number;
            /**
             * Format: int64
             * @description Their size in bytes.
             */
            reclaimed_bytes: number;
        };
        /**
         * @description Where a task is.
         * @enum {string}
         */
        TaskPhase: "preparing" | "working" | "awaiting_review" | "done" | "failed" | "cancelled";
        /** @description A task. */
        TaskResource: {
            /** @description The change its commits form, once it has one. */
            change?: string | null;
            /** @description The commit it started from, once settled. */
            commit?: string | null;
            /**
             * Format: date-time
             * @description When it was created.
             */
            created_at: string;
            /** @description Why it failed. */
            error?: string | null;
            /** @description What the agent is asked to do. */
            goal: string;
            /** @description Its id (`task_...`). */
            id: string;
            /** @description Where it is. */
            phase: components["schemas"]["TaskPhase"];
            /** @description The repository. */
            repo: string;
            /** @description Its sandbox, once started. */
            sandbox?: string | null;
            /** @description The tool, once settled. */
            tool?: string | null;
            /** @description Its turns, oldest first. */
            turns: components["schemas"]["TurnResource"][];
        };
        /** @description A text frame the client sends. */
        TerminalClientFrame: {
            /**
             * Format: int32
             * @description Columns.
             */
            cols: number;
            /**
             * Format: int32
             * @description Rows.
             */
            rows: number;
            /** @enum {string} */
            type: "resize";
        };
        /**
         * @description Why a terminal ended without an exit code.
         * @enum {string}
         */
        TerminalEndReason: "sandbox_unavailable" | "execution_error" | "closed" | "lost";
        /** @description A text frame the server sends. */
        TerminalServerFrame: {
            /**
             * Format: int32
             * @description The process's exit code (128 + signal when killed by a signal).
             */
            code?: number | null;
            failure?: components["schemas"]["TerminalEndReason"] | null;
            /** @enum {string} */
            type: "exit";
        };
        /** @description An entry of a task's transcript. */
        TranscriptEntry: {
            /** @description What happened. */
            item: components["schemas"]["TranscriptItem"];
            /**
             * Format: int32
             * @description Its position in the transcript, from 0.
             */
            position: number;
            /**
             * Format: int32
             * @description The turn it belongs to, from 1.
             */
            turn: number;
        };
        /** @description One thing a tool did or said. */
        TranscriptItem: {
            /** @enum {string} */
            kind: "message";
            /** @description The text. */
            text: string;
        } | {
            /** @description The call's id, matching its result. */
            id: string;
            /** @description Its input, as JSON. */
            input: string;
            /** @enum {string} */
            kind: "tool_call";
            /** @description The tool called, such as `Bash`. */
            name: string;
        } | {
            /** @description The call's id. */
            id: string;
            /** @description Whether the call failed. */
            is_error: boolean;
            /** @enum {string} */
            kind: "tool_result";
            /** @description The output. */
            output: string;
        } | {
            /** @enum {string} */
            kind: "output";
            /** @description The line. */
            text: string;
        } | {
            /** @enum {string} */
            kind: "error";
            /** @description What it said. */
            text: string;
        };
        /** @description A task's transcript from a position on. */
        TranscriptResource: {
            /** @description The entries, oldest first. */
            entries: components["schemas"]["TranscriptEntry"][];
            /** @description Whether no turn runs, so no entries follow until another turn starts. */
            idle: boolean;
            /**
             * Format: int32
             * @description The position to read from next.
             */
            next: number;
        };
        /** @description One turn of a task. */
        TurnResource: {
            /** @description The job running the tool, whose logs are its output. */
            job: string;
            /**
             * Format: int32
             * @description Its position, from 1.
             */
            number: number;
            /** @description What the tool was asked. */
            prompt: string;
            /** @description How it failed. */
            reason?: string | null;
            /** @description Where it is. */
            status: components["schemas"]["TurnStatus"];
        };
        /**
         * @description Where a turn is.
         * @enum {string}
         */
        TurnStatus: "running" | "succeeded" | "failed";
        /** @description A snapshot a repository recorded under a key: a warm snapshot, or an agent snapshot over one. */
        WarmSnapshotResource: {
            /**
             * Format: date-time
             * @description When it was recorded, from the event log; absent when the log no longer holds it.
             */
            built_at?: string | null;
            /** @description The commit it was built at. */
            commit: string;
            /** @description What it was built from: a digest of its base, build command and lockfiles. */
            key: string;
            /**
             * Format: date-time
             * @description When a run's checkout or a task's sandbox was last made over it, at hour precision; absent
             *     until a use is recorded.
             */
            last_used_at?: string | null;
            /**
             * Format: int64
             * @description The bytes of its layer blobs; absent until the blob store reports sizes.
             */
            size_bytes?: number | null;
            /** @description The snapshot's id. */
            snapshot: string;
        };
        /** @description What the sandboxes placed on a worker hold of it, by their limits. */
        WorkerAllocation: {
            /**
             * Format: int64
             * @description Their memory limits, in MiB, summed.
             */
            memory_mib: number;
            /**
             * Format: int64
             * @description Their CPU limits, in millicpus, summed.
             */
            millicpus: number;
            /**
             * Format: int32
             * @description Sandboxes that are not stopped or failed.
             */
            sandboxes: number;
        };
        /** @description What a worker offers. */
        WorkerCapabilities: {
            /** @description The CPU architecture: `x86_64` or `aarch64`. */
            arch: string;
            /** @description The operating system: `linux` or `macos`. */
            os: string;
            /** @description The worker protocol version it speaks. */
            protocol: string;
            /** @description The sandbox runtimes it offers: `process`, `oci` or `firecracker`. */
            runtimes: string[];
        };
        /**
         * @description Whether a worker's stream to the server is open.
         * @enum {string}
         */
        WorkerConnection: "connected" | "disconnected" | "lost";
        /** @description A worker. */
        WorkerResource: {
            /** @description What its sandboxes hold. */
            allocated: components["schemas"]["WorkerAllocation"];
            /** @description What it offers. */
            capabilities: components["schemas"]["WorkerCapabilities"];
            /** @description Its connection. */
            connection: components["schemas"]["WorkerConnection"];
            /**
             * Format: date-time
             * @description When its stream closed, while disconnected.
             */
            disconnected_since?: string | null;
            /** @description Its id (`wrk_...`). */
            id: string;
            /** @description Its labels. */
            labels: {
                [key: string]: string;
            };
            /** @description Whether new work may be placed on it by spec. */
            schedulability: components["schemas"]["WorkerSchedulability"];
            /** @description Whether new work may be placed on it now: connected and schedulable. */
            schedulable: boolean;
            usage?: components["schemas"]["WorkerUsage"] | null;
        };
        /**
         * @description Whether new work may be placed on a worker.
         * @enum {string}
         */
        WorkerSchedulability: "schedulable" | "draining";
        /** @description What a worker last reported holding of its machine. */
        WorkerUsage: {
            /**
             * Format: int64
             * @description Free bytes of that file system.
             */
            disk_free_bytes: number;
            /**
             * Format: int64
             * @description Total bytes of the file system holding its data directory.
             */
            disk_total_bytes: number;
            /**
             * Format: int64
             * @description Bytes its layer cache holds.
             */
            layer_cache_bytes: number;
            /**
             * Format: int64
             * @description The budget above which its unpinned layers are evicted.
             */
            layer_cache_limit_bytes: number;
            /**
             * Format: date-time
             * @description When the server received the report.
             */
            reported_at: string;
            /**
             * Format: int32
             * @description Sandboxes it holds, starting or ready.
             */
            sandboxes: number;
        };
        /**
         * @description Where a workspace is.
         * @enum {string}
         */
        WorkspacePhase: "starting" | "running" | "stopping" | "stopped";
        /** @description A workspace. */
        WorkspaceResource: {
            /** @description The branch it was opened on. */
            branch: string;
            /**
             * Format: date-time
             * @description When it was created.
             */
            created_at: string;
            /** @description Its id (`wsp_...`). */
            id: string;
            /**
             * Format: date-time
             * @description When a person last used it.
             */
            last_activity: string;
            /** @description The person it belongs to. */
            owner: string;
            /** @description Where it is. */
            phase: components["schemas"]["WorkspacePhase"];
            /** @description The repository. */
            repo: string;
            /** @description Its sandbox, while it has one. Its terminal is the sandbox's. */
            sandbox?: string | null;
            /** @description The snapshot its last stop sealed, which holds everything the workspace has. */
            snapshot?: string | null;
        };
    };
    responses: never;
    parameters: never;
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export interface operations {
    getBlob: {
        parameters: {
            query?: {
                /** @description Presigned URL expiry, Unix seconds */
                expires?: number;
                /** @description Presigned URL signature */
                signature?: string;
            };
            header?: never;
            path: {
                digest: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/octet-stream": number[];
                };
            };
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    putBlob: {
        parameters: {
            query?: {
                /** @description Presigned URL expiry, Unix seconds */
                expires?: number;
                /** @description Presigned URL signature */
                signature?: string;
            };
            header?: never;
            path: {
                /** @description `blake3:<64 hex>` */
                digest: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/octet-stream": number[];
            };
        };
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            413: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    getChange: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ChangeResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    approveChange: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ApproveRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ChangeResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    closeChange: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ChangeResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    commentOnChange: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CommentRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ChangeResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    getChangeDiff: {
        parameters: {
            query?: {
                /** @description The revision, from 1; the latest when omitted. */
                revision?: number;
            };
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DiffResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    mergeChange: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ChangeResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    requestChanges: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ChangeResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    reviseChange: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ChangeResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    listChangeRuns: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RunResource"][];
                };
            };
        };
    };
    stream: {
        parameters: {
            query?: {
                /** @description Only events of this repository's resources */
                repo?: string;
                /** @description Resume after this sequence; 0 replays the whole log; the current head when absent */
                after?: number;
            };
            header?: {
                /** @description Resume after this sequence when `after` is absent */
                "Last-Event-ID"?: number | null;
            };
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "text/event-stream": components["schemas"]["EventNotice"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    getJob: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["JobResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    getJobLogs: {
        parameters: {
            query?: never;
            header?: {
                /** @description Resume after this event */
                "Last-Event-ID"?: number | null;
            };
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "text/event-stream": string;
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    listRepos: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RepoResource"][];
                };
            };
        };
    };
    registerRepo: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RegisterRepoRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RepoResource"];
                };
            };
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    getRepo: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RepoResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    listRepoChanges: {
        parameters: {
            query?: {
                /** @description Only changes at this phase; repeatable */
                phase?: components["schemas"]["ChangePhase"][];
                /** @description `newest` lists the newest first */
                order?: components["schemas"]["ListOrder"];
            };
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ChangeResource"][];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    openChange: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["OpenChangeRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ChangeResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    listRepoRuns: {
        parameters: {
            query?: {
                /** @description `next_cursor` of the previous page */
                cursor?: string;
                /** @description Page size, 1 to 200; 50 when omitted */
                limit?: number;
            };
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RunList"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    listRepoSecrets: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SecretList"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    setRepoSecret: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                /** @description `[A-Z_][A-Z0-9_]*` */
                name: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "text/plain": string;
            };
        };
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    deleteRepoSecret: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
                name: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    listRepoSnapshots: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WarmSnapshotResource"][];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    snapshotRepo: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RepoSnapshotRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RepoSnapshotResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    listRepoTasks: {
        parameters: {
            query?: {
                /** @description Only tasks at this phase; repeatable */
                phase?: components["schemas"]["TaskPhase"][];
                /** @description `newest` lists the newest first */
                order?: components["schemas"]["ListOrder"];
            };
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TaskResource"][];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    createTask: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateTaskRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TaskResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    createWorkspace: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateWorkspaceRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WorkspaceResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    getRun: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RunResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    listSandboxes: {
        parameters: {
            query?: {
                /** @description `key=value`; every label must match */
                label?: string[];
                /** @description `next_cursor` of the previous page */
                cursor?: string;
                /** @description Page size, 1 to 200; 50 when omitted */
                limit?: number;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SandboxList"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    createSandbox: {
        parameters: {
            query?: never;
            header?: {
                /** @description Makes retries safe */
                "Idempotency-Key"?: string | null;
            };
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateSandboxRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SandboxResource"];
                };
            };
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    getSandbox: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SandboxResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    execInSandbox: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ExecRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["JobResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    createSeal: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            202: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SealResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    stopSandbox: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            202: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SandboxResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    openTerminal: {
        parameters: {
            query?: {
                /** @description The program and its arguments, one value each; the default shell when omitted */
                command?: string[];
                /** @description Initial columns, 1 to 65535; 80 when omitted */
                cols?: number;
                /** @description Initial rows, 1 to 65535; 24 when omitted */
                rows?: number;
            };
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Switching protocols to the WebSocket subprotocol `igloo.terminal.v1` */
            101: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    getSeal: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SealResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    uploadSealLayer: {
        parameters: {
            query: {
                /** @description Presigned URL expiry, Unix seconds */
                expires: number;
                /** @description Presigned URL signature */
                signature: string;
                /** @description The layer's digest, `blake3:<64 hex>` */
                digest: string;
                /** @description `full` for a whole root file system */
                layer?: string;
            };
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/octet-stream": number[];
            };
        };
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            413: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    createSnapshot: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateSnapshotRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SnapshotResource"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    importImage: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ImportImageRequest"];
            };
        };
        responses: {
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SnapshotResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    getSnapshot: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SnapshotResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    getStorage: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["StorageResource"];
                };
            };
        };
    };
    getTask: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TaskResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    cancelTask: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TaskResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    getTaskTranscript: {
        parameters: {
            query?: {
                /** @description The first position to return; 0 when absent. */
                after?: number;
            };
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["TranscriptResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    listWorkers: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WorkerResource"][];
                };
            };
        };
    };
    listWorkspaces: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WorkspaceResource"][];
                };
            };
        };
    };
    getWorkspace: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WorkspaceResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    deleteWorkspace: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    startWorkspace: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WorkspaceResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
    stopWorkspace: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WorkspaceResource"];
                };
            };
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Problem"];
                };
            };
        };
    };
}
