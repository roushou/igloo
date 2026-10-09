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

  /**
   * How long before `now` a timestamp was: `just now`, `5m ago`, `3h ago`, `2d ago`, and the date
   * from 30 days on, with the year when it is not the current one. A timestamp in the future
   * reads `just now`.
   */
  relative(iso: string, now: number): string {
    const seconds = Math.floor((now - new Date(iso).getTime()) / 1000);
    if (seconds < 45) return "just now";
    const minutes = Math.round(seconds / 60);
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.round(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.round(hours / 24);
    if (days < 30) return `${days}d ago`;
    const then = new Date(iso);
    const sameYear = then.getFullYear() === new Date(now).getFullYear();
    return then.toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
      year: sameYear ? undefined : "numeric",
    });
  },

  /** `acme/shop` for `https://forge.example.com/acme/shop.git`; other locations pass through. */
  repoName(location: string): string {
    const path = location.replace(/\.git$/, "").replace(/\/+$/, "");
    const parts = path.split(/[/:]/).filter(Boolean);
    return parts.length >= 2 ? parts.slice(-2).join("/") : path;
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

  /** `prefix_abcd…wxyz` for a long id and seven characters for a commit; the rest pass through. */
  shortId(id: string): string {
    if (/^[0-9a-f]{40}$/.test(id)) return id.slice(0, 7);
    const underscore = id.indexOf("_");
    if (id.length <= 14 || underscore === -1) return id;
    return `${id.slice(0, underscore + 1)}${id.slice(underscore + 1, underscore + 5)}…${id.slice(-4)}`;
  },
} as const;
