// Small shared formatters (orchestrator-owned; add freely, don't change behaviour).

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes)) return "—";
  const gb = bytes / 1024 ** 3;
  if (gb >= 1) return `${gb >= 10 ? gb.toFixed(0) : gb.toFixed(1)} GB`;
  const mb = bytes / 1024 ** 2;
  if (mb >= 1) return `${mb.toFixed(0)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

export function formatGb(gb: number | null | undefined): string {
  if (gb == null || !Number.isFinite(gb)) return "—";
  return `${gb >= 10 ? gb.toFixed(0) : gb.toFixed(1).replace(/\.0$/, "")} GB`;
}

export function formatCount(n: number | null | undefined): string {
  if (n == null) return "—";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1).replace(/\.0$/, "")}k`;
  return String(n);
}
