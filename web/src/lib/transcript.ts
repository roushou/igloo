import type { TranscriptEntry } from "@/api/client";
import type { components } from "@/api/schema.gen";

type Turn = components["schemas"]["TurnResource"];

/** One line to show for a tool call: its result, if it came back, rides along. */
export type ToolRow = {
  kind: "tool";
  key: string;
  name: string;
  input: string;
  result: { output: string; isError: boolean } | null;
  /** How long the call took; null until the server reports when entries happened. */
  durationMs: number | null;
};

/** What the transcript view lists, in order. */
export type Row =
  | { kind: "turn"; key: string; number: number; prompt: string | null; reason: string | null }
  | { kind: "message" | "output" | "error"; key: string; text: string }
  | ToolRow;

/** One side of an edit: the removed and the added text. */
export type Edit = { key: string; path: string; removed: string; added: string };

/**
 * The rows of a transcript: each turn starts with its prompt, a tool call and its result are one
 * row, and the rest are one row per entry.
 */
export function transcriptRows(entries: readonly TranscriptEntry[], turns: readonly Turn[]): Row[] {
  const rows: Row[] = [];
  const calls = new Map<string, ToolRow>();
  let turn = 0;
  for (const { position, turn: number, item } of entries) {
    while (turn < number) {
      turn += 1;
      const known = turns.find((candidate) => candidate.number === turn);
      rows.push({
        kind: "turn",
        key: `turn-${turn}`,
        number: turn,
        prompt: known?.prompt ?? null,
        reason: known?.status === "failed" ? (known.reason ?? null) : null,
      });
    }
    switch (item.kind) {
      case "message":
      case "output":
      case "error":
        rows.push({ kind: item.kind, key: `e-${position}`, text: item.text });
        break;
      case "tool_call": {
        const row: ToolRow = {
          kind: "tool",
          key: `e-${position}`,
          name: item.name,
          input: item.input,
          result: null,
          durationMs: null,
        };
        calls.set(item.id, row);
        rows.push(row);
        break;
      }
      case "tool_result": {
        const call = calls.get(item.id);
        if (call) {
          call.result = { output: item.output, isError: item.is_error };
        } else {
          rows.push({
            kind: "tool",
            key: `e-${position}`,
            name: "result",
            input: "",
            result: { output: item.output, isError: item.is_error },
            durationMs: null,
          });
        }
        break;
      }
    }
  }
  return rows;
}

function parse(input: string): Record<string, unknown> | null {
  try {
    const value: unknown = JSON.parse(input);
    return value && typeof value === "object" ? (value as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

const SUMMARY_FIELDS = ["command", "file_path", "path", "pattern", "url", "description", "query"];

/** The one line a collapsed tool call shows: its most telling argument, first line only. */
export function toolSummary(input: string): string {
  const fields = parse(input);
  if (!fields) return input.split("\n")[0] ?? "";
  for (const name of SUMMARY_FIELDS) {
    const value = fields[name];
    if (typeof value === "string" && value) return value.split("\n")[0] ?? "";
  }
  return "";
}

/**
 * The edits a tool call makes, when its input has the shape of an edit (`old_string` and
 * `new_string`, a list of them, or a whole-file `content`); empty for any other call.
 */
export function toolEdits(input: string): Edit[] {
  const fields = parse(input);
  if (!fields) return [];
  const path = typeof fields.file_path === "string" ? fields.file_path : "";
  const edit = (value: Record<string, unknown>, at: number): Edit | null =>
    typeof value.new_string === "string"
      ? {
          key: `${path}:${at}`,
          path,
          removed: typeof value.old_string === "string" ? value.old_string : "",
          added: value.new_string,
        }
      : null;
  if (Array.isArray(fields.edits)) {
    return fields.edits.flatMap((value, at) => {
      const found =
        value && typeof value === "object" ? edit(value as Record<string, unknown>, at) : null;
      return found ? [found] : [];
    });
  }
  const single = edit(fields, 0);
  if (single) return [single];
  if (typeof fields.content === "string" && path) {
    return [{ key: `${path}:content`, path, removed: "", added: fields.content }];
  }
  return [];
}
