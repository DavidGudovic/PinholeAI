// Full-window image viewer: wheel/pinch zoom toward the cursor, drag to pan,
// double-click toggles fit / 100%, arrows move between images, Esc closes.
// Shows the in-memory blob URL only; nothing is written anywhere.
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import {
  ChevronLeft,
  ChevronRight,
  Minus,
  Plus,
  ScanSearch,
  X,
} from "lucide-react";
import { IconButton, useEscape, useModalFocus } from "./ui";

export interface ViewerImage {
  url: string;
  width: number;
  height: number;
  alt: string;
}

interface View {
  scale: number;
  x: number; // offset of the image centre from the stage centre, in screen px
  y: number;
}

const MAX_ZOOM = 16;

export function clampView(
  v: View,
  stage: { w: number; h: number },
  img: { w: number; h: number },
  fit: number,
): View {
  const scale = Math.min(Math.max(v.scale, fit), Math.max(MAX_ZOOM, fit));
  const limit = (size: number, view: number) =>
    Math.max(0, (size * scale - view) / 2);
  const lx = limit(img.w, stage.w);
  const ly = limit(img.h, stage.h);
  return {
    scale,
    x: Math.min(lx, Math.max(-lx, v.x)),
    y: Math.min(ly, Math.max(-ly, v.y)),
  };
}

/** Zoom to `next`, keeping the image point under (cx, cy) (relative to the stage centre) fixed. */
export function zoomAt(v: View, next: number, cx: number, cy: number): View {
  const k = next / v.scale;
  return { scale: next, x: cx - (cx - v.x) * k, y: cy - (cy - v.y) * k };
}

export function ImageViewer({
  images,
  index,
  onIndex,
  onClose,
}: {
  images: ViewerImage[];
  index: number;
  onIndex: (i: number) => void;
  onClose: () => void;
}) {
  useEscape(true, onClose);
  // Focus moves into the viewer while it is open and back to where it was on close.
  const root = useRef<HTMLDivElement>(null);
  useModalFocus(true, root);
  const img = images[index];
  const stage = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [view, setView] = useState<View>({ scale: 1, x: 0, y: 0 });
  const viewRef = useRef(view);
  viewRef.current = view;
  const fit =
    size.w > 0 && img ? Math.min(size.w / img.width, size.h / img.height) : 1;
  const fitRef = useRef(fit);
  fitRef.current = fit;
  const dims = { w: img?.width ?? 1, h: img?.height ?? 1 };
  const dimsRef = useRef(dims);
  dimsRef.current = dims;
  const sizeRef = useRef(size);
  sizeRef.current = size;

  const apply = useCallback(
    (v: View) =>
      setView(clampView(v, sizeRef.current, dimsRef.current, fitRef.current)),
    [],
  );

  useLayoutEffect(() => {
    const el = stage.current;
    if (!el) return;
    const measure = () => setSize({ w: el.clientWidth, h: el.clientHeight });
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // New image or new window size: back to fit.
  useEffect(() => {
    setView({ scale: fit, x: 0, y: 0 });
  }, [index, fit]);

  const zoomBy = useCallback(
    (factor: number, cx = 0, cy = 0) => {
      const v = viewRef.current;
      apply(zoomAt(v, v.scale * factor, cx, cy));
    },
    [apply],
  );

  const rel = (clientX: number, clientY: number) => {
    const r = stage.current!.getBoundingClientRect();
    return {
      x: clientX - r.left - r.width / 2,
      y: clientY - r.top - r.height / 2,
    };
  };

  // Wheel needs a non-passive listener to stop the page scrolling.
  useEffect(() => {
    const el = stage.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const p = rel(e.clientX, e.clientY);
      // Trackpad pinch arrives as ctrl+wheel with small deltas.
      const speed = e.ctrlKey ? 0.01 : 0.0015;
      zoomBy(Math.exp(-e.deltaY * speed), p.x, p.y);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [zoomBy]);

  const go = useCallback(
    (d: number) => {
      const n = index + d;
      if (n >= 0 && n < images.length) onIndex(n);
    },
    [index, images.length, onIndex],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowLeft") go(-1);
      else if (e.key === "ArrowRight") go(1);
      else if (e.key === "+" || e.key === "=") zoomBy(1.25);
      else if (e.key === "-") zoomBy(0.8);
      else if (e.key === "0") apply({ scale: fitRef.current, x: 0, y: 0 });
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [go, zoomBy, apply]);

  // Pointers: one drags, two pinch.
  const pointers = useRef(new Map<number, { x: number; y: number }>());
  const gesture = useRef<{ dist: number } | null>(null);
  const moved = useRef(false);
  const onPointerDown = (e: React.PointerEvent) => {
    stage.current?.setPointerCapture(e.pointerId);
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
    moved.current = false;
    if (pointers.current.size === 2) {
      const [a, b] = [...pointers.current.values()];
      gesture.current = { dist: Math.hypot(a.x - b.x, a.y - b.y) };
    }
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const prev = pointers.current.get(e.pointerId);
    if (!prev) return;
    const cur = { x: e.clientX, y: e.clientY };
    pointers.current.set(e.pointerId, cur);
    if (pointers.current.size === 2 && gesture.current) {
      const [a, b] = [...pointers.current.values()];
      const dist = Math.hypot(a.x - b.x, a.y - b.y);
      const mid = rel((a.x + b.x) / 2, (a.y + b.y) / 2);
      if (gesture.current.dist > 0)
        zoomBy(dist / gesture.current.dist, mid.x, mid.y);
      gesture.current.dist = dist;
      moved.current = true;
    } else if (pointers.current.size === 1) {
      const dx = cur.x - prev.x;
      const dy = cur.y - prev.y;
      if (dx || dy) moved.current = true;
      const v = viewRef.current;
      apply({ ...v, x: v.x + dx, y: v.y + dy });
    }
  };
  const onPointerUp = (e: React.PointerEvent) => {
    pointers.current.delete(e.pointerId);
    gesture.current = null;
  };

  const toggle = (e: React.MouseEvent) => {
    const v = viewRef.current;
    const p = rel(e.clientX, e.clientY);
    const atFit = Math.abs(v.scale - fitRef.current) < 1e-3;
    // Already 100% or fit is larger than 100%: go back to fit.
    if (!atFit || fitRef.current >= 1)
      apply({ scale: fitRef.current, x: 0, y: 0 });
    else apply(zoomAt(v, 1, p.x, p.y));
  };

  if (!img) return null;
  const percent = Math.round(view.scale * 100);
  return createPortal(
    <div
      ref={root}
      role="dialog"
      aria-modal="true"
      aria-label="Image viewer"
      tabIndex={-1}
      className="fixed inset-0 z-[60] flex flex-col bg-neutral-950 text-white outline-none"
    >
      <div className="flex shrink-0 items-center justify-between gap-2 px-3 py-2">
        <span className="text-xs text-neutral-400 tabular-nums">
          {img.width}×{img.height} · {percent}%
          {images.length > 1 && ` · ${index + 1} of ${images.length}`}
        </span>
        <div className="flex items-center gap-1">
          <IconButton label="Zoom out" size="sm" onClick={() => zoomBy(0.8)}>
            <Minus className="h-4 w-4" />
          </IconButton>
          <IconButton label="Zoom in" size="sm" onClick={() => zoomBy(1.25)}>
            <Plus className="h-4 w-4" />
          </IconButton>
          <IconButton
            label="Fit to window"
            size="sm"
            onClick={() => apply({ scale: fit, x: 0, y: 0 })}
          >
            <ScanSearch className="h-4 w-4" />
          </IconButton>
          <IconButton label="Close" size="sm" onClick={onClose}>
            <X className="h-4 w-4" />
          </IconButton>
        </div>
      </div>
      <div
        ref={stage}
        className="relative min-h-0 flex-1 touch-none overflow-hidden select-none"
        style={{ cursor: view.scale > fit + 1e-3 ? "grab" : "default" }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onDoubleClick={toggle}
        onClick={(e) => {
          // A click on the empty backdrop (not a drag) closes.
          if (moved.current || e.target !== e.currentTarget) return;
          // The image ignores pointer events, so test against its rectangle.
          const p = rel(e.clientX, e.clientY);
          const v = viewRef.current;
          const d = dimsRef.current;
          const inside =
            Math.abs(p.x - v.x) <= (d.w * v.scale) / 2 &&
            Math.abs(p.y - v.y) <= (d.h * v.scale) / 2;
          if (!inside) onClose();
        }}
      >
        {size.w > 0 && (
          <img
            src={img.url}
            alt={img.alt}
            draggable={false}
            className="pointer-events-none absolute top-1/2 left-1/2 max-w-none"
            style={{
              width: img.width,
              height: img.height,
              transform: `translate(-50%, -50%) translate(${view.x}px, ${view.y}px) scale(${view.scale})`,
              imageRendering: view.scale >= 3 ? "pixelated" : "auto",
              willChange: "transform",
            }}
          />
        )}
        {index > 0 && (
          <button
            type="button"
            aria-label="Previous image"
            onPointerDown={(e) => e.stopPropagation()}
            onClick={() => go(-1)}
            className="absolute top-1/2 left-3 -translate-y-1/2 rounded-full bg-white/10 p-2 hover:bg-white/20"
          >
            <ChevronLeft className="h-5 w-5" />
          </button>
        )}
        {index < images.length - 1 && (
          <button
            type="button"
            aria-label="Next image"
            onPointerDown={(e) => e.stopPropagation()}
            onClick={() => go(1)}
            className="absolute top-1/2 right-3 -translate-y-1/2 rounded-full bg-white/10 p-2 hover:bg-white/20"
          >
            <ChevronRight className="h-5 w-5" />
          </button>
        )}
      </div>
    </div>,
    document.body,
  );
}
