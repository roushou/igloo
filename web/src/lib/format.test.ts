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
    expect(format.shortId("c".repeat(40))).toBe("ccccccc");
  });

  it("says how long ago a time was", () => {
    const now = new Date("2026-01-10T12:00:00Z").getTime();
    const ago = (ms: number) => new Date(now - ms).toISOString();
    expect(format.relative(ago(10_000), now)).toBe("just now");
    expect(format.relative(ago(5 * 60_000), now)).toBe("5m ago");
    expect(format.relative(ago(3 * 3600_000), now)).toBe("3h ago");
    expect(format.relative(ago(2 * 86_400_000), now)).toBe("2d ago");
    expect(format.relative(ago(-60_000), now)).toBe("just now");
    expect(format.relative(ago(20 * 86_400_000), now)).toBe("20d ago");
    // From 30 days on a date: with the year only when it is not the current one.
    expect(format.relative(ago(40 * 86_400_000), now)).toContain("2025");
    const june = new Date("2026-06-10T12:00:00Z").getTime();
    expect(format.relative(new Date(june - 40 * 86_400_000).toISOString(), june)).not.toContain(
      "20",
    );
  });

  it("names a repository by its owner and name", () => {
    expect(format.repoName("https://forge.example.com/acme/shop.git")).toBe("acme/shop");
    expect(format.repoName("git@forge.example.com:acme/shop")).toBe("acme/shop");
    expect(format.repoName("shop")).toBe("shop");
  });
});
