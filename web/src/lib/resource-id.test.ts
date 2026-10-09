import { ResourceId } from "./resource-id";

const SUFFIX = "01j9z3k4m5n6p7q8r9s0t1v2w3";

describe("ResourceId", () => {
  it.each(["repo", "chg", "task", "run", "job", "sbx"])("parses %s ids", (prefix) => {
    const id = ResourceId.parse(`  ${prefix}_${SUFFIX}\n`);
    expect(id?.kind).toBe(prefix);
    expect(id?.id).toBe(`${prefix}_${SUFFIX}`);
  });

  it.each(["", "task_short", `usr_${SUFFIX}`, `task-${SUFFIX}`, `TASK_${SUFFIX}`])(
    "rejects %j",
    (text) => {
      expect(ResourceId.parse(text)).toBeNull();
    },
  );
});
