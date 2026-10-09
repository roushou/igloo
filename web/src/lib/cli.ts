/** The `igloo` command lines equivalent to each console action on a change. */
export const changeCli = {
  comment(id: string, body: string, where?: { revision?: number; path?: string; line?: number }) {
    const flags = [
      where?.revision !== undefined ? `--revision ${where.revision}` : "",
      where?.path ? `--path ${where.path}` : "",
      where?.line !== undefined ? `--line ${where.line}` : "",
    ].filter(Boolean);
    return [`igloo change comment ${id}`, JSON.stringify(body || "<comment>"), ...flags].join(" ");
  },
  requestChanges: (id: string) => `igloo change request-changes ${id}`,
  approve: (id: string) => `igloo change approve ${id}`,
  merge: (id: string) => `igloo change merge ${id}`,
  close: (id: string) => `igloo change close ${id}`,
  revise: (id: string) => `igloo change push ${id}`,
} as const;
