// Plain-word labels and small pure helpers shared by Models, Settings and First run.
import type { DownloadState, GroupStatus, HardwareView } from "../../../lib/types";
import { formatBytes, formatGb } from "../../../lib/format";

/** Engine backend in plain words (First run, Settings hints). */
export function backendPlain(backend: string | null | undefined): string {
  switch (backend) {
    case "cuda":
      return "NVIDIA graphics cards (CUDA)";
    case "vulkan":
      return "AMD, Intel and other graphics cards (Vulkan)";
    case "cpu":
      return "your processor only — no graphics card (slow)";
    default:
      return "your computer";
  }
}

/** Short backend name for Settings. */
export function backendShort(backend: string | null | undefined): string {
  switch (backend) {
    case "cuda":
      return "CUDA";
    case "vulkan":
      return "Vulkan";
    case "cpu":
      return "CPU";
    default:
      return backend ?? "—";
  }
}

/** Hardware tier (registry hardware_profiles) in plain words. */
export function tierPlain(tier: string | null | undefined): string {
  switch (tier) {
    case "low":
      return "Entry level (under 8 GB)";
    case "mid":
      return "Mid-range (8–12 GB)";
    case "high":
      return "High-end (13–20 GB)";
    case "ultra":
      return "Top tier (21 GB and up)";
    default:
      return tier ?? "—";
  }
}

/** "RTX 5070 Ti (16 GB)" / "No GPU found". */
export function gpuSummary(hw: Pick<HardwareView, "gpu" | "vramGb"> | null): string {
  if (!hw) return "—";
  if (!hw.gpu) return "No GPU — using the processor";
  return `${hw.gpu.name} (${formatGb(hw.vramGb)})`;
}

/** Quant in plain words for recommendation cards. */
export function quantPlain(quant: string | null | undefined): string | null {
  if (!quant) return null;
  const q = quant.toLowerCase();
  if (q === "bf16" || q === "fp16") return "Full quality version";
  if (q.startsWith("q8") || q === "fp8") return "High-quality compact version";
  if (q.startsWith("q4") || q.startsWith("q5") || q.startsWith("q3")) return "Smaller version for your GPU";
  return null;
}

export function stateLabel(state: DownloadState): string {
  switch (state) {
    case "queued":
      return "Waiting…";
    case "downloading":
      return "Downloading";
    case "verifying":
      return "Checking files…";
    case "done":
      return "Done";
    case "failed":
      return "Failed";
    case "cancelled":
      return "Cancelled";
  }
}

export function isActive(g: Pick<GroupStatus, "state">): boolean {
  return g.state === "queued" || g.state === "downloading" || g.state === "verifying";
}

/** 0…1 progress of a group (0 when the total is unknown). */
export function groupFraction(g: Pick<GroupStatus, "downloadedBytes" | "totalBytes" | "state">): number {
  if (g.state === "done" || g.state === "verifying") return 1;
  if (!g.totalBytes || g.totalBytes <= 0) return 0;
  return Math.min(1, Math.max(0, g.downloadedBytes / g.totalBytes));
}

/** "3.1 GB of 12 GB · 26%" */
export function progressText(g: Pick<GroupStatus, "downloadedBytes" | "totalBytes" | "state">): string {
  if (g.state === "verifying") return "Checking files…";
  if (!g.totalBytes) return g.downloadedBytes ? `${formatBytes(g.downloadedBytes)}` : "Starting…";
  return `${formatBytes(g.downloadedBytes)} of ${formatBytes(g.totalBytes)} · ${Math.floor(groupFraction(g) * 100)}%`;
}

/** Relative "last used" in plain words. `now` (ms) injectable for tests. Rust sends Unix seconds; ms also accepted. */
export function lastUsedText(ts: number | null | undefined, now: number = Date.now()): string {
  if (ts == null || !Number.isFinite(ts) || ts <= 0) return "Never";
  // Rust sends seconds (InstalledModel.lastUsed); milliseconds are accepted too.
  const ms = ts < 1e12 ? ts * 1000 : ts;
  const diff = now - ms;
  if (diff < 0) return "Just now";
  const min = 60_000;
  const hour = 60 * min;
  const day = 24 * hour;
  if (diff < 2 * min) return "Just now";
  if (diff < hour) return `${Math.floor(diff / min)} min ago`;
  if (diff < day) return `${Math.floor(diff / hour)} h ago`;
  if (diff < 2 * day) return "Yesterday";
  if (diff < 30 * day) return `${Math.floor(diff / day)} days ago`;
  if (diff < 365 * day) {
    const m = Math.floor(diff / (30 * day));
    return m <= 1 ? "A month ago" : `${m} months ago`;
  }
  return "Over a year ago";
}

/** Percentage for the thumbs-up ratio (API gives 0…1 or 0…100). */
export function ratioPercent(r: number | null | undefined): string | null {
  if (r == null || !Number.isFinite(r)) return null;
  const pct = r <= 1 ? r * 100 : r;
  return `${Math.round(pct)}%`;
}

/** File format as shown to people. */
export function formatLabel(format: string): string {
  const f = format.toLowerCase();
  if (f === "safetensor" || f === "safetensors") return "SafeTensors";
  if (f === "gguf") return "GGUF";
  return format;
}

/** MIME type from image magic bytes, so preview Blobs render reliably. */
export function sniffImageType(bytes: Uint8Array): string {
  const b = bytes;
  if (b.length >= 8 && b[0] === 0x89 && b[1] === 0x50 && b[2] === 0x4e && b[3] === 0x47) return "image/png";
  if (b.length >= 3 && b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff) return "image/jpeg";
  if (b.length >= 12 && b[0] === 0x52 && b[1] === 0x49 && b[2] === 0x46 && b[3] === 0x46 && b[8] === 0x57 && b[9] === 0x45 && b[10] === 0x42 && b[11] === 0x50)
    return "image/webp";
  if (b.length >= 4 && b[0] === 0x47 && b[1] === 0x49 && b[2] === 0x46 && b[3] === 0x38) return "image/gif";
  if (b.length >= 12 && b[4] === 0x66 && b[5] === 0x74 && b[6] === 0x79 && b[7] === 0x70 && b[8] === 0x61 && b[9] === 0x76 && b[10] === 0x69 && b[11] === 0x66)
    return "image/avif";
  return "application/octet-stream";
}

/** File name from a full path (Windows or POSIX). */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/** Lower-case extension check for "Add a file I already have". */
export function isModelFile(path: string): boolean {
  return /\.(safetensors|gguf)$/i.test(path);
}
