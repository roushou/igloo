/** The kinds of resource the console can open by id, keyed by TypeID prefix. */
export type ResourceKind = "repo" | "chg" | "task" | "run" | "job" | "sbx";

const KINDS: readonly ResourceKind[] = ["repo", "chg", "task", "run", "job", "sbx"];

/** A pasted resource id: `<prefix>_<26 characters>` with one of the known prefixes. */
export class ResourceId {
  private static readonly PATTERN = /^([a-z]+)_([0-9a-z]{26})$/;

  private constructor(
    readonly kind: ResourceKind,
    readonly id: string,
  ) {}

  /** The id in `text`, ignoring surrounding whitespace; `null` when it is not a known id. */
  static parse(text: string): ResourceId | null {
    const id = text.trim();
    const match = ResourceId.PATTERN.exec(id);
    const kind = KINDS.find((kind) => kind === match?.[1]);
    return kind ? new ResourceId(kind, id) : null;
  }
}
