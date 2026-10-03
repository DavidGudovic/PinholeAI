// Drop / paste / pick an image. HTML5 drag & drop (Tauri's native drag-drop is
// disabled), clipboard paste of image files, and a file picker. Bytes go
// straight to Rust (`import_image`); nothing is written to disk. A picture dropped
// where no drop area takes it is handled like a pasted one.
import { useEffect, useRef, useState, type DragEvent, type ReactNode } from "react";
import { ImagePlus } from "lucide-react";
import { clipboardImage } from "../lib/api";
import { isTauri } from "../lib/mock";
import { onStrayDrop } from "../lib/platform";
import { imageFromTransfer } from "../lib/state/images";
import { modKey } from "../lib/state/platform";
import { Button, cx, focusRing } from "./ui";

/** Hidden file input + opener. */
export function useFilePicker(onFile: (f: File) => void) {
  const input = useRef<HTMLInputElement>(null);
  const el = (
    <input
      ref={input}
      type="file"
      accept="image/png,image/jpeg,image/webp,image/*"
      className="hidden"
      tabIndex={-1}
      onChange={(e) => {
        const f = e.target.files?.[0];
        e.target.value = "";
        if (f) onFile(f);
      }}
    />
  );
  return { open: () => input.current?.click(), input: el };
}

/**
 * Listen for image paste (Ctrl/Cmd+V) while `active` (the tab is visible).
 * Text pastes into inputs are left alone.
 */
export function useImagePaste(active: boolean, onFile: (f: File) => void) {
  const cb = useRef(onFile);
  useEffect(() => {
    cb.current = onFile;
  });
  useEffect(() => {
    if (!active) return;
    const onPaste = (e: ClipboardEvent) => {
      // A dialog over the tab owns the paste.
      if (document.querySelector("[role=dialog]")) return;
      // Text copied from Word or a web page often carries a picture too: pasting it into a
      // text field stays a text paste. (A file copied in a file manager also has its path
      // as text, with a uri-list: that one is an image paste.)
      const t = e.target as HTMLElement | null;
      const editable = !!t && (t.tagName === "TEXTAREA" || t.tagName === "INPUT" || !!t.isContentEditable);
      const types = Array.from(e.clipboardData?.types ?? []);
      if (editable && types.includes("text/plain") && !types.includes("text/uri-list")) return;
      const f = imageFromTransfer(e.clipboardData);
      if (f) {
        e.preventDefault();
        cb.current(f);
        return;
      }
      // The Linux WebView (WebKitGTK) gives the paste event no data at all, even with a picture
      // on the clipboard: read it through the app. In a text box only a picture without text.
      if (types.length === 0 && isTauri()) {
        void clipboardImage(editable)
          .then((p) => p && cb.current(p))
          .catch(() => undefined);
      }
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, [active]);
}

/** What happened when something was dropped where no drop area takes it. */
export type StrayDrop = { kind: "picture"; file: File } | { kind: "not-a-picture" } | { kind: "link" } | { kind: "ignored" };

/** Reads a drop that no drop area took: a picture file, another file, or a web link with no file. */
export function readStrayDrop(dt: DataTransfer): StrayDrop {
  const f = imageFromTransfer(dt);
  if (f) return { kind: "picture", file: f };
  const types = Array.from(dt.types ?? []);
  if (types.includes("Files")) return { kind: "not-a-picture" };
  // A picture dragged out of a web browser can arrive as its address only (Linux). When the page
  // holds the picture itself in a data: address, it is read from there; nothing is downloaded.
  const url = types.includes("text/uri-list") ? dt.getData("text/uri-list") : "";
  const html = types.includes("text/html") ? dt.getData("text/html") : "";
  // The page's markup only when no address came with it: with one, the markup's src can be a
  // placeholder for the picture actually shown.
  const fromHtml = url.trim() ? "" : (/<img\b[^>]*?\ssrc\s*=\s*["']?(data:[^"'\s>]+)/i.exec(html)?.[1] ?? "");
  const inline = pictureFromDataUrl(url.trim()) ?? pictureFromDataUrl(fromHtml);
  if (inline) return { kind: "picture", file: inline };
  if (/^(?:https?|data):/im.test(url)) return { kind: "link" };
  return { kind: "ignored" };
}

/** Same limit as adding a picture (Rust refuses bigger ones anyway). */
const MAX_INLINE_BYTES = 64 * 1024 * 1024;

/** A PNG, JPEG or WebP held in a base64 data: address, or null. */
export function pictureFromDataUrl(url: string): File | null {
  const m = /^data:(image\/(?:png|jpe?g|webp))(?:;[^;,]*)*;base64,([A-Za-z0-9+/=\s]+)$/i.exec(url);
  if (!m || (m[2].length * 3) / 4 > MAX_INLINE_BYTES) return null;
  try {
    const bin = atob(m[2].replace(/\s+/g, ""));
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    const type = m[1].toLowerCase().replace("jpg", "jpeg");
    return new File([bytes], `dropped.${type.slice(6).replace("jpeg", "jpg")}`, { type });
  } catch {
    return null;
  }
}

/**
 * Pictures dropped anywhere in the window outside a drop area (and not while a dialog is open,
 * like paste). `onOther` gets the drops that carry no picture.
 */
export function useImageDrop(onFile: (f: File) => void, onOther: (d: Exclude<StrayDrop, { kind: "picture" }>) => void) {
  const cb = useRef({ onFile, onOther });
  useEffect(() => {
    cb.current = { onFile, onOther };
  });
  useEffect(
    () =>
      onStrayDrop((dt) => {
        if (document.querySelector("[role=dialog]")) return;
        const d = readStrayDrop(dt);
        if (d.kind === "picture") cb.current.onFile(d.file);
        else cb.current.onOther(d);
      }),
    [],
  );
}

type Offer = (f: File) => void;
const offers = new Set<Offer>();

/** Hands a dropped picture to the paste chooser, from a drop area that didn't use it. */
export function offerDroppedPicture(f: File) {
  // Like paste and other drops: not while a dialog (the full-size viewer, the chooser) is open.
  if (document.querySelector("[role=dialog]")) return;
  offers.forEach((o) => o(f));
}

/** Pictures handed over with `offerDroppedPicture`. */
export function useOfferedPicture(onFile: Offer) {
  const cb = useRef(onFile);
  useEffect(() => {
    cb.current = onFile;
  });
  useEffect(() => {
    const o: Offer = (f) => cb.current(f);
    offers.add(o);
    return () => {
      offers.delete(o);
    };
  }, []);
}

/** Wraps children in a drop target; shows an overlay while dragging an image over it. */
export function DropTarget({ onFile, children, className = "", label = "Drop the image here" }: { onFile: (f: File) => void; children: ReactNode; className?: string; label?: string }) {
  const [over, setOver] = useState(false);
  const depth = useRef(0);
  const hasFiles = (e: DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes("Files");
  return (
    <div
      className={cx("relative", className)}
      onDragEnter={(e) => {
        if (!hasFiles(e)) return;
        e.preventDefault();
        depth.current++;
        setOver(true);
      }}
      onDragOver={(e) => {
        if (!hasFiles(e)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "copy";
      }}
      onDragLeave={() => {
        depth.current = Math.max(0, depth.current - 1);
        if (!depth.current) setOver(false);
      }}
      onDrop={(e) => {
        if (!hasFiles(e)) return;
        e.preventDefault();
        depth.current = 0;
        setOver(false);
        const f = imageFromTransfer(e.dataTransfer);
        if (f) onFile(f);
      }}
    >
      {children}
      {over && (
        <div className="pinhole-fade pointer-events-none absolute inset-2 z-20 flex items-center justify-center rounded-2xl border-2 border-dashed border-amber-500 bg-amber-50/85 text-sm font-medium text-amber-900 dark:bg-amber-950/80 dark:text-amber-100">
          {label}
        </div>
      )}
    </div>
  );
}

/** Big empty-state drop zone. */
export function DropZone({ onFile, title, busy, children }: { onFile: (f: File) => void; title: string; busy?: boolean; children?: ReactNode }) {
  const picker = useFilePicker(onFile);
  return (
    <div className="flex h-full min-h-72 w-full flex-col items-center justify-center rounded-2xl border-2 border-dashed border-neutral-300 bg-white/60 p-8 text-center dark:border-neutral-700 dark:bg-neutral-900/40">
      <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-amber-100 text-amber-700 dark:bg-amber-500/15 dark:text-amber-300">
        <ImagePlus className="h-7 w-7" />
      </div>
      <h2 className="text-base font-semibold">{title}</h2>
      <p className="mt-1 max-w-sm text-sm text-neutral-500">
        Drop an image here, paste one with <span className="font-medium">{modKey}+V</span>, or choose a file.
      </p>
      <Button variant="primary" className={cx("mt-5", focusRing)} onClick={picker.open} disabled={busy}>
        <ImagePlus className="h-4 w-4" /> Choose an image
      </Button>
      {picker.input}
      {children}
    </div>
  );
}
