import { format } from "./format";

describe("format", () => {
  it("shows durations by their two largest units", () => {
    expect(format.duration(12_000)).toBe("12s");
    expect(format.duration(185_000)).toBe("3m 05s");
    expect(format.duration(3_720_000)).toBe("1h 02m");
    expect(format.duration(-5)).toBe("0s");
  });

  it("shows sizes in binary units", () => {
    expect(format.bytes(9)).toBe("9 B");
    expect(format.bytes(1536)).toBe("1.5 KiB");
    expect(format.bytes(6 * 1024 ** 3)).toBe("6.0 GiB");
    expect(format.mebibytes(4096)).toBe("4.0 GiB");
    expect(format.cpus(1500)).toBe("1.5 CPUs");
    expect(format.cpus(1000)).toBe("1 CPU");
  });

  it("shortens long ids and leaves short ones", () => {
    expect(format.shortId("task_01j9z3k4m5n6p7q8r9s0t1v2w3")).toBe("task_01j9…v2w3");
    expect(format.shortId("abc")).toBe("abc");
  });
});
