// Session images → in-memory blob: URLs. The WebView never loads images from
// the network or disk: bytes come from Rust (`get_image`) and live in RAM.
import { discardImage, getImage, importImage } from "../api";
import type { AppState, ImgRef } from "./model";
import type { Store } from "./store";

/** get_image returns PNG for every session image (imported JPEG/WebP are re-encoded in Rust); sniffing is a fallback. */
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
    for (const r of refs) {
      URL.revokeObjectURL(r.url);
      if (r.thumbUrl) URL.revokeObjectURL(r.thumbUrl);
    }
  }, 0);
  if (discard) for (const r of refs) void discardImage(r.id).catch(() => undefined);
}

/** Long side of a thumbnail in pixels (about twice the largest tile that shows one). */
export const THUMB_SIZE = 160;

/** What a small tile shows: the ref's thumbnail once it is made, else the full picture. */
export const thumbSrc = (r: ImgRef): string => r.thumbUrl ?? r.url;

/** A small PNG copy of the ref's picture as a blob: URL; null when the picture is already small or can't be drawn. */
export async function makeThumbUrl(r: ImgRef): Promise<string | null> {
  const long = Math.max(r.width, r.height);
  if (long <= THUMB_SIZE || typeof document === "undefined") return null;
  try {
    const img = new Image();
    if (typeof img.decode !== "function") return null;
    img.src = r.url;
    await img.decode();
    const scale = THUMB_SIZE / long;
    const c = document.createElement("canvas");
    c.width = Math.max(1, Math.round(r.width * scale));
    c.height = Math.max(1, Math.round(r.height * scale));
    const ctx = c.getContext("2d");
    if (!ctx) return null;
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(img, 0, 0, c.width, c.height);
    const blob = await new Promise<Blob | null>((done) => c.toBlob(done, "image/png"));
    return blob ? URL.createObjectURL(blob) : null;
  } catch {
    return null;
  }
}

/** Thumbnails made at the same time: each one decodes the full picture on the main thread. */
export const THUMBS_AT_ONCE = 1;

/**
 * Make a thumbnail for each ref once it is in the store, so a picture never waits for its
 * thumbnail. Refs are queued and made THUMBS_AT_ONCE at a time; a ref that left the store while
 * queued is skipped. A thumbnail whose ref is gone by the time it is ready is revoked straight
 * away; otherwise releaseRefs revokes it together with the ref's url.
 */
export function watchThumbs(store: Store, make: (r: ImgRef) => Promise<string | null> = makeThumbUrl): () => void {
  const tried = new WeakSet<ImgRef>();
  const queue: ImgRef[] = [];
  let running = 0;
  let seen: AppState["images"] | null = null;
  const pump = () => {
    while (running < THUMBS_AT_ONCE && queue.length) {
      const r = queue.shift()!;
      if (store.getState().images[r.id]?.url !== r.url) continue;
      running++;
      void make(r)
        .catch(() => null)
        .then((thumbUrl) => {
          if (thumbUrl) {
            store.dispatch({ type: "setThumb", id: r.id, url: r.url, thumbUrl });
            if (store.getState().images[r.id]?.thumbUrl !== thumbUrl) URL.revokeObjectURL(thumbUrl);
          }
          running--;
          pump();
        });
    }
  };
  const scan = () => {
    const images = store.getState().images;
    if (images === seen) return;
    seen = images;
    for (const r of Object.values(images)) {
      if (r.thumbUrl || tried.has(r)) continue;
      tried.add(r);
      queue.push(r);
    }
    pump();
  };
  scan();
  return store.subscribe(scan);
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
