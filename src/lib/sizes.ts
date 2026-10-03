// Named sizes for Create's Fine-tune drawer ("My screen", "Phone", "Instagram", "Thumbnail").
// Each one keeps the model's usual picture area (its Square size) and only changes the shape,
// rounded to multiples of 64 like the Width and Height fields.
import type { FamilyUi } from "./types";

export type NamedSizeId = "screen" | "phone" | "instagram" | "thumbnail";

export interface NamedSize {
  id: NamedSizeId;
  label: string;
  /** width / height */
  ratio: number;
  /** Short words for the tooltip. */
  note: string;
}

export const MIN_SIDE = 256;
export const MAX_SIDE = 4096;

/** The fixed shapes, then "My screen" when the screen's size is known. */
export function namedSizes(screen: { width: number; height: number } | null): NamedSize[] {
  const out: NamedSize[] = [];
  if (screen && screen.width > 0 && screen.height > 0) {
    out.push({ id: "screen", label: "My screen", ratio: screen.width / screen.height, note: "your screen’s shape" });
  }
  out.push(
    { id: "phone", label: "Phone", ratio: 9 / 16, note: "9:16, for a phone wallpaper or a story" },
    { id: "instagram", label: "Instagram", ratio: 4 / 5, note: "4:5, for an Instagram post" },
    { id: "thumbnail", label: "Thumbnail", ratio: 16 / 9, note: "16:9, for a video thumbnail" },
  );
  return out;
}

const snap = (n: number) => Math.min(MAX_SIDE, Math.max(MIN_SIDE, Math.round(n / 64) * 64));

/** Longest side over shortest side that a picture can be made at (MAX_ASPECT in the Rust size rules). */
export const MAX_ASPECT = 3;

/** Width and height for `ratio` (kept within 1:3 … 3:1) with about the same area as the model's Square shape. */
export function sizeForRatio(ratio: number, ui: Pick<FamilyUi, "shapes"> | null): [number, number] {
  ratio = Math.min(MAX_ASPECT, Math.max(1 / MAX_ASPECT, ratio));
  const [sw, sh] = ui?.shapes.square ?? [1024, 1024];
  const area = sw * sh;
  const w = snap(Math.sqrt(area * ratio));
  const h = snap(Math.sqrt(area / ratio));
  // Rounding can push the shape past MAX_ASPECT; trim the long side back to it.
  const cap = (long: number, short: number) => Math.min(long, Math.floor((short * MAX_ASPECT) / 64) * 64);
  return w >= h ? [cap(w, h), h] : [w, cap(h, w)];
}

/**
 * Create's size for "Same as reference": the reference picture's shape at the model's usual area,
 * or null when that shape isn't in use. Width and Height typed in Fine-tune still win.
 */
export function referenceSize(
  c: { refShape: boolean; refImageId: string | null },
  ref: { width: number; height: number } | null | undefined,
  ui: Pick<FamilyUi, "shapes"> | null,
): [number, number] | null {
  if (!c.refShape || !c.refImageId || !ref || !(ref.width > 0 && ref.height > 0)) return null;
  return sizeForRatio(ref.width / ref.height, ui);
}

/**
 * Width and height for "Same as reference": the reference size, or, when one side was typed in
 * Fine-tune, the other side worked out from the reference's shape (rounded to 64).
 */
export function fillReferenceSize(width: number | null | undefined, height: number | null | undefined, ref: [number, number]): [number, number] {
  const round64 = (n: number) => Math.max(64, Math.round(n / 64) * 64);
  if (width != null && height != null) return [width, height];
  if (width != null) return [width, round64((width * ref[1]) / ref[0])];
  if (height != null) return [round64((height * ref[0]) / ref[1]), height];
  return ref;
}

/** The monitor's size in real pixels, or null when the WebView doesn't say. */
export function screenPixels(): { width: number; height: number } | null {
  try {
    const s = window.screen;
    const dpr = window.devicePixelRatio || 1;
    const width = Math.round(s.width * dpr);
    const height = Math.round(s.height * dpr);
    return width > 0 && height > 0 ? { width, height } : null;
  } catch {
    return null;
  }
}
