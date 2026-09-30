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
    out.push({ id: "screen", label: "My screen", ratio: screen.width / screen.height, note: `${screen.width}×${screen.height}` });
  }
  out.push(
    { id: "phone", label: "Phone", ratio: 9 / 16, note: "9:16, for a phone wallpaper or a story" },
    { id: "instagram", label: "Instagram", ratio: 4 / 5, note: "4:5, for an Instagram post" },
    { id: "thumbnail", label: "Thumbnail", ratio: 16 / 9, note: "16:9, for a video thumbnail" },
  );
  return out;
}

const snap = (n: number) => Math.min(MAX_SIDE, Math.max(MIN_SIDE, Math.round(n / 64) * 64));

/** Width and height for `ratio` with about the same area as the model's Square shape. */
export function sizeForRatio(ratio: number, ui: Pick<FamilyUi, "shapes"> | null): [number, number] {
  const [sw, sh] = ui?.shapes.square ?? [1024, 1024];
  const area = sw * sh;
  return [snap(Math.sqrt(area * ratio)), snap(Math.sqrt(area / ratio))];
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
