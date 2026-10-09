/** Display formats for the numbers and times the console shows. */
export const format = {
  /** `1h 02m`, `3m 05s`, `12s`: the largest two units of `ms`, never negative. */
  duration(ms: number): string {
    const total = Math.max(0, Math.floor(ms / 1000));
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const seconds = total % 60;
    if (hours > 0) return `${hours}h ${String(minutes).padStart(2, "0")}m`;
    if (minutes > 0) return `${minutes}m ${String(seconds).padStart(2, "0")}s`;
    return `${seconds}s`;
  },

  /** `1.5 GiB`, `320 MiB`, `12 KiB`, `9 B`. */
  bytes(bytes: number): string {
    const units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let value = bytes;
    let unit = 0;
    while (value >= 1024 && unit < units.length - 1) {
      value /= 1024;
      unit += 1;
    }
    return `${unit === 0 || value >= 100 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
  },

  /** `1.5 CPUs` from millicpus. */
  cpus(millicpus: number): string {
    const value = millicpus / 1000;
    return `${Number.isInteger(value) ? value : value.toFixed(1)} CPU${value === 1 ? "" : "s"}`;
  },

  /** `12 MiB` from mebibytes. */
  mebibytes(mib: number): string {
    return format.bytes(mib * 1024 * 1024);
  },

  /** A short local date and time for a timestamp. */
  time(iso: string): string {
    return new Date(iso).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  },

  /** `prefix_abcd…wxyz` for a long id; short ids pass through. */
  shortId(id: string): string {
    const underscore = id.indexOf("_");
    if (id.length <= 14 || underscore === -1) return id;
    return `${id.slice(0, underscore + 1)}${id.slice(underscore + 1, underscore + 5)}…${id.slice(-4)}`;
  },
} as const;
