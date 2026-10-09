import type { Change } from "@/api/client";
import type { State } from "./status";

/** One rule a change must meet to merge, and whether it does. */
export type Rule = {
  id: "checks" | "approval" | "fast_forward";
  met: boolean;
  /** What the rule says now, whether met or not. */
  text: string;
  /** How to show it: orange only when the rule waits on the user. */
  state: State;
};

/** The merge rules from the change's readiness; empty when the server sent none. */
export function mergeRules(change: Pick<Change, "readiness" | "target_branch">): Rule[] {
  const readiness = change.readiness;
  if (!readiness) return [];
  const checks: Record<typeof readiness.checks, [string, State]> = {
    passed: ["Checks passed", "passed"],
    failed: ["A check failed", "failed"],
    running: ["Checks are still running", "running"],
    missing: ["No checks have run for the latest revision", "closed"],
  };
  const paths = readiness.approval.protected_paths ?? [];
  const approval: Record<typeof readiness.approval.state, [string, State]> = {
    not_required: ["No approval required", "passed"],
    given: ["Approved", "passed"],
    required: [
      `A human must approve it: it touches ${paths.join(", ") || "protected paths"}`,
      "needs-you",
    ],
  };
  const [checksText, checksState] = checks[readiness.checks];
  const [approvalText, approvalState] = approval[readiness.approval.state];
  return [
    { id: "checks", met: readiness.checks === "passed", text: checksText, state: checksState },
    {
      id: "approval",
      met: readiness.approval.state !== "required",
      text: approvalText,
      state: approvalState,
    },
    {
      id: "fast_forward",
      met: readiness.fast_forward,
      text: readiness.fast_forward
        ? `Up to date with ${change.target_branch}`
        : `${change.target_branch} has moved: record a new revision after rebasing`,
      state: readiness.fast_forward ? "passed" : "failed",
    },
  ];
}

/** Why the change cannot merge now, or `null` when it can. */
export function mergeBlocker(change: Change): string | null {
  if (change.phase !== "open") return `The change is already ${change.phase}`;
  if (!change.readiness) return "Merge readiness is not known yet";
  return mergeRules(change).find((rule) => !rule.met)?.text ?? null;
}

/** Why an open change waits on the user, in a line. */
export function waitingOn(change: Change): string {
  const approval = change.readiness?.approval;
  if (approval?.state === "required") {
    return `Needs your approval: touches ${(approval.protected_paths ?? []).join(", ") || "protected paths"}`;
  }
  return "Checks passed: ready to merge";
}
