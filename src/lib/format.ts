// Small shared formatters.

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes)) return "—";
  // Decimal units, matching how download sizes are published (HF, CivitAI, disks).
  const gb = bytes / 1e9;
  if (gb >= 1) return `${gb >= 10 ? gb.toFixed(0) : gb.toFixed(1)} GB`;
  const mb = bytes / 1e6;
  if (mb >= 1) return `${mb.toFixed(0)} MB`;
  return `${Math.max(1, Math.round(bytes / 1e3))} KB`;
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

/** Words for "Needs ~X GB" + the Fits / Tight / Too big badge (SPEC §6.2). */
export interface FitWords {
  /** "Needs ~6 GB VRAM" / "Needs ~3 GB memory" (+ " (estimate)" for GPU estimates). */
  need: string;
  /** Compact form for pickers: "~6 GB VRAM" / "~3 GB RAM". */
  short: string;
  /** Badge text, or null when there is no fit. */
  badge: string | null;
  tone: "green" | "amber" | "red";
  /** Tooltip. */
  title: string;
}

/**
 * Without a usable GPU (`vram.onCpu`) the figure is system RAM and "tight" means
 * "runs on the processor — slow", so the words say that instead of "VRAM".
 */
export function fitWords(vram: { gb: number; estimate: boolean; onCpu?: boolean }, fit: "fits" | "tight" | "tooBig" | null | undefined): FitWords {
  const gb = formatGb(vram.gb);
  const tone = fit === "fits" ? "green" : fit === "tight" ? "amber" : "red";
  if (vram.onCpu) {
    return {
      need: `Needs ~${gb} memory`,
      short: `~${gb} RAM`,
      badge: fit === "fits" ? "Fits" : fit === "tight" ? "Slow" : fit === "tooBig" ? "Too big" : null,
      tone,
      title:
        fit === "tooBig"
          ? `No graphics card found, and this is too big to run on the processor (needs ~${gb} of memory)`
          : `Runs on the processor — slow. Needs ~${gb} of memory (RAM).`,
    };
  }
  const estimate = vram.estimate ? " (estimate)" : "";
  return {
    need: `Needs ~${gb} VRAM${estimate}`,
    short: `~${gb} VRAM`,
    badge: fit === "fits" ? "Fits" : fit === "tight" ? "Tight" : fit === "tooBig" ? "Too big" : null,
    tone,
    title: `Needs ~${gb} VRAM${estimate}`,
  };
}
