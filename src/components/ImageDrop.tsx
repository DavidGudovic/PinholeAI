// Drop / paste / pick an image. HTML5 drag & drop (Tauri's native drag-drop is
// disabled), clipboard paste of image files, and a file picker. Bytes go
// straight to Rust (`import_image`); nothing is written to disk.
import { useEffect, useRef, useState, type DragEvent, type ReactNode } from "react";
import { ImagePlus } from "lucide-react";
import { clipboardImage } from "../lib/api";
import { isTauri } from "../lib/mock";
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
