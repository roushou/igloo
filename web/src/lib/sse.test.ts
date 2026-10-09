import { SseParser } from "./sse";

describe("SseParser", () => {
  it("parses frames split across chunks and skips keep-alive comments", () => {
    const parser = new SseParser();
    expect(parser.feed(": keep-alive\n\nid: 4\nevent: igloo.run.passed\nda")).toEqual([]);
    expect(parser.feed('ta: {"a":1}\n\nid: 5\nevent: x\ndata: y\n\n')).toEqual([
      { id: "4", event: "igloo.run.passed", data: '{"a":1}' },
      { id: "5", event: "x", data: "y" },
    ]);
  });

  it("handles CRLF line ends and joins multi-line data", () => {
    const parser = new SseParser();
    expect(parser.feed("data: a\r\ndata: b\r\n\r\n")).toEqual([
      { id: null, event: "message", data: "a\nb" },
    ]);
  });
});
