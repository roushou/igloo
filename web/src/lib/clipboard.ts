/** Writes to the system clipboard, which the browser may refuse. */
export const clipboard = {
  /** Copies `text`; false when the browser denied it, instead of an unhandled rejection. */
  async copy(text: string): Promise<boolean> {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      return false;
    }
  },
};
