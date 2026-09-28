// Session images → in-memory blob: URLs. The WebView never loads images from
// the network or disk: bytes come from Rust (`get_image`) and live in RAM.
import { discardImage, getImage, importImage } from "../api";
import type { ImgRef } from "./model";

/** get_image returns PNG for generated images, but imported ones keep their bytes (PNG/JPEG/WebP). */
export function imageMime(buf: ArrayBuffer): string {
  const b = new Uint8Array(buf, 0, Math.min(12, buf.byteLength));
  if (b[0] === 0xff && b[1] === 0xd8) return "image/jpeg";
  if (b[0] === 0x52 && b[1] === 0x49 && b[2] === 0x46 && b[3] === 0x46 && b[8] === 0x57 && b[9] === 0x45 && b[10] === 0x42 && b[11] === 0x50) return "image/webp";
  return "image/png";
}

export async function refFromSession(id: string, width: number, height: number): Promise<ImgRef> {
  const buf = await getImage(id);
  const blob = new Blob([buf], { type: imageMime(buf) });
  return { id, url: URL.createObjectURL(blob), width, height };
}

/** Put a dropped / pasted / picked image into the Rust session and get a ref for it. */
export async function importBlob(blob: Blob): Promise<ImgRef> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  const im = await importImage(bytes);
  return refFromSession(im.id, im.width, im.height);
}

/** Revoke blob URLs (after the current frame) and, unless the whole session was cleared, discard the Rust copies. */
export function releaseRefs(refs: ImgRef[], discard: boolean): void {
  setTimeout(() => {
    for (const r of refs) URL.revokeObjectURL(r.url);
  }, 0);
  if (discard) for (const r of refs) void discardImage(r.id).catch(() => undefined);
}

/** First image file in a DataTransfer (drop or paste), if any. */
export function imageFromTransfer(dt: DataTransfer | null): File | null {
  if (!dt) return null;
  for (const f of Array.from(dt.files ?? [])) if (f.type.startsWith("image/")) return f;
  for (const item of Array.from(dt.items ?? [])) {
    if (item.kind === "file" && item.type.startsWith("image/")) {
      const f = item.getAsFile();
      if (f) return f;
    }
  }
  return null;
}
