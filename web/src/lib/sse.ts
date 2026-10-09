/** One server-sent event. */
export type SseFrame = { id: string | null; event: string; data: string };

/**
 * Parses a server-sent event stream from text chunks. Comments (keep-alives) and blocks without
 * data produce no frame; `event` defaults to `message`.
 */
export class SseParser {
  private buffer = "";
  private id: string | null = null;

  /** Feeds the next chunk and returns the frames it completes. */
  feed(chunk: string): SseFrame[] {
    this.buffer += chunk;
    const frames: SseFrame[] = [];
    for (;;) {
      const end = /\r?\n\r?\n/.exec(this.buffer);
      if (!end) return frames;
      const block = this.buffer.slice(0, end.index);
      this.buffer = this.buffer.slice(end.index + end[0].length);
      const frame = this.parse(block);
      if (frame) frames.push(frame);
    }
  }

  private parse(block: string): SseFrame | null {
    let event = "message";
    const data: string[] = [];
    for (const line of block.split(/\r?\n/)) {
      if (line === "" || line.startsWith(":")) continue;
      const colon = line.indexOf(":");
      const field = colon === -1 ? line : line.slice(0, colon);
      let value = colon === -1 ? "" : line.slice(colon + 1);
      if (value.startsWith(" ")) value = value.slice(1);
      if (field === "id") this.id = value;
      else if (field === "event") event = value;
      else if (field === "data") data.push(value);
    }
    return data.length === 0 ? null : { id: this.id, event, data: data.join("\n") };
  }
}
