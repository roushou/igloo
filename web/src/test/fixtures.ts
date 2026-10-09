import type { components } from "@/api/schema.gen";
import { REPO } from "./render-app";

type S = components["schemas"];

/** A well-formed id: `prefix_` and 26 characters ending in `n`. */
export function id(prefix: string, n: number): string {
  return `${prefix}_${String(n).padStart(26, "0")}`;
}

const T0 = "2026-01-01T10:00:00Z";

export function task(n: number, overrides: Partial<S["TaskResource"]> = {}): S["TaskResource"] {
  return {
    id: id("task", n),
    repo: REPO.id,
    goal: `Goal of task ${n}`,
    phase: "working",
    created_at: T0,
    turns: [{ job: id("job", n), number: 1, prompt: `Goal of task ${n}`, status: "running" }],
    tool: "claude-code",
    sandbox: id("sbx", n),
    ...overrides,
  };
}

export function readiness(overrides: Partial<S["MergeReadiness"]> = {}): S["MergeReadiness"] {
  return {
    checks: "passed",
    approval: { state: "not_required", protected_paths: [] },
    fast_forward: true,
    ...overrides,
  };
}

export function change(
  n: number,
  overrides: Partial<S["ChangeResource"]> = {},
): S["ChangeResource"] {
  return {
    id: id("chg", n),
    repo: REPO.id,
    title: `Change ${n}`,
    phase: "open",
    source_branch: `igloo/task-${n}`,
    target_branch: "main",
    revisions: [{ number: 1, base: "a".repeat(40), head: "b".repeat(40), created_at: T0 }],
    approvals: [],
    comments: [],
    readiness: readiness(),
    ...overrides,
  };
}

export function run(n: number, overrides: Partial<S["RunResource"]> = {}): S["RunResource"] {
  return {
    id: id("run", n),
    change: id("chg", n),
    revision: 1,
    commit: "b".repeat(40),
    phase: "checking",
    started_at: T0,
    checks: [
      { name: "lint", status: "passed", job: id("job", 100 + n), exit_code: 0 },
      { name: "test", status: "started", job: id("job", 200 + n) },
    ],
    ...overrides,
  };
}

export function job(n: number, overrides: Partial<S["JobResource"]> = {}): S["JobResource"] {
  return {
    id: id("job", n),
    sandbox: id("sbx", n),
    argv: ["cargo", "test"],
    env: {},
    phase: "running",
    submitted_at: T0,
    timeout_seconds: 600,
    ...overrides,
  };
}

export function sandbox(
  n: number,
  overrides: Partial<S["SandboxResource"]> = {},
): S["SandboxResource"] {
  return {
    id: id("sbx", n),
    created_at: T0,
    desired: "running",
    phase: "running",
    env: {},
    labels: {},
    generation: 1,
    isolation: "container",
    limits: { memory_mib: 4096, millicpus: 2000 },
    network: "deny_all",
    snapshot: id("snap", n),
    repo: REPO.id,
    worker: id("wrk", 1),
    ...overrides,
  };
}

export function worker(
  n: number,
  overrides: Partial<S["WorkerResource"]> = {},
): S["WorkerResource"] {
  const GIB = 1024 ** 3;
  return {
    id: id("wrk", n),
    labels: { zone: "home" },
    connection: "connected",
    schedulability: "schedulable",
    schedulable: true,
    capabilities: { arch: "x86_64", os: "linux", protocol: "1", runtimes: ["oci"] },
    allocated: { millicpus: 3000, memory_mib: 6144, sandboxes: 2 },
    usage: {
      disk_total_bytes: 100 * GIB,
      disk_free_bytes: 40 * GIB,
      layer_cache_bytes: 6 * GIB,
      layer_cache_limit_bytes: 20 * GIB,
      reported_at: T0,
      sandboxes: 2,
    },
    ...overrides,
  };
}

export function transcript(
  entries: S["TranscriptItem"][],
  overrides: Partial<S["TranscriptResource"]> = {},
  from = 0,
  turn = 1,
): S["TranscriptResource"] {
  return {
    entries: entries.map((item, i) => ({ position: from + i, turn, item })),
    next: from + entries.length,
    idle: false,
    ...overrides,
  };
}

export function file(
  path: string,
  overrides: Partial<S["FileDiffResource"]> = {},
): S["FileDiffResource"] {
  return {
    path,
    status: "modified",
    additions: 1,
    deletions: 1,
    binary: false,
    truncated: false,
    patch: `--- a/${path}\n+++ b/${path}\n@@ -1,2 +1,2 @@\n context\n-old line\n+new line\n`,
    ...overrides,
  };
}

export function diff(files: S["FileDiffResource"][], revision = 1): S["DiffResource"] {
  return { base: "a".repeat(40), head: "b".repeat(40), revision, files };
}
