// "Only change here" brush. Paints on a canvas at the image's real resolution
// (CSS-scaled over the image); exported as a white-on-black PNG of the same size.
import { useEffect, useImperativeHandle, useRef, useState, type PointerEvent, type Ref } from "react";

export interface MaskHandle {
  clear(): void;
  /** White = change. Null when nothing is painted. */
  exportPng(): Promise<Blob | null>;
}

const PAINT = "rgba(245, 158, 11, 1)"; // amber-500, shown at reduced opacity

/** Canvas area (image pixels) that holds every paint stroke so far. */
interface Box {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

/**
 * Whether any pixel inside `box` is painted (nothing is painted outside it), reading only that
 * area. Anti-aliased eraser edges can leave a few nearly transparent pixels.
 */
function hasPaint(c: HTMLCanvasElement, box: Box | null): boolean {
  const ctx = c.getContext("2d");
  if (!ctx || !c.width || !c.height || !box) return false;
  const x0 = Math.max(0, Math.floor(box.x0));
  const y0 = Math.max(0, Math.floor(box.y0));
  const x1 = Math.min(c.width, Math.ceil(box.x1));
  const y1 = Math.min(c.height, Math.ceil(box.y1));
  if (!(x1 > x0 && y1 > y0)) return false;
  const px = ctx.getImageData(x0, y0, x1 - x0, y1 - y0).data;
  for (let i = 3; i < px.length; i += 4) {
    if (px[i] > 16) return true;
  }
  return false;
}

export function MaskCanvas({
  ref,
  width,
  height,
  displayWidth,
  brush,
  erase,
  active,
  onPaintedChange,
}: {
  ref?: Ref<MaskHandle>;
  width: number;
  height: number;
  /** Rendered width in CSS px (to convert the brush size). */
  displayWidth: number;
  /** Brush diameter in CSS px. */
  brush: number;
  erase: boolean;
  active: boolean;
  onPaintedChange: (painted: boolean) => void;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const last = useRef<{ x: number; y: number } | null>(null);
  const painted = useRef(false);
  const paintBox = useRef<Box | null>(null);
  const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null);

  // New image size → start over.
  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    c.width = width;
    c.height = height;
    painted.current = false;
    paintBox.current = null;
    onPaintedChange(false);
  }, [width, height]);

  useImperativeHandle(ref, () => ({
    clear() {
      const c = canvas.current;
      c?.getContext("2d")?.clearRect(0, 0, c.width, c.height);
      painted.current = false;
      paintBox.current = null;
      onPaintedChange(false);
    },
    async exportPng() {
      const c = canvas.current;
      // Everything erased again: nothing painted (an all-black mask would change nothing).
      if (!c || !painted.current || !hasPaint(c, paintBox.current)) return null;
      const tmp = document.createElement("canvas");
      tmp.width = c.width;
      tmp.height = c.height;
      const t = tmp.getContext("2d")!;
      t.drawImage(c, 0, 0);
      t.globalCompositeOperation = "source-in";
      t.fillStyle = "#fff";
      t.fillRect(0, 0, tmp.width, tmp.height);
      const out = document.createElement("canvas");
      out.width = c.width;
      out.height = c.height;
      const o = out.getContext("2d")!;
      o.fillStyle = "#000";
      o.fillRect(0, 0, out.width, out.height);
      o.drawImage(tmp, 0, 0);
      return new Promise<Blob | null>((res) => out.toBlob((b) => res(b), "image/png"));
    },
  }));

  const toImage = (e: PointerEvent<HTMLCanvasElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    return { x: ((e.clientX - r.left) / r.width) * width, y: ((e.clientY - r.top) / r.height) * height };
  };

  const stroke = (from: { x: number; y: number }, to: { x: number; y: number }) => {
    const ctx = canvas.current?.getContext("2d");
    if (!ctx) return;
    const scale = displayWidth ? width / displayWidth : 1;
    ctx.globalCompositeOperation = erase ? "destination-out" : "source-over";
    ctx.strokeStyle = PAINT;
    ctx.fillStyle = PAINT;
    ctx.lineWidth = brush * scale;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.beginPath();
    ctx.moveTo(from.x, from.y);
    ctx.lineTo(to.x, to.y);
    ctx.stroke();
    if (!erase) {
      // Half the line width plus a pixel for anti-aliased edges.
      const r = (brush * scale) / 2 + 1;
      const b = paintBox.current;
      paintBox.current = {
        x0: Math.min(b?.x0 ?? Infinity, from.x - r, to.x - r),
        y0: Math.min(b?.y0 ?? Infinity, from.y - r, to.y - r),
        x1: Math.max(b?.x1 ?? -Infinity, from.x + r, to.x + r),
        y1: Math.max(b?.y1 ?? -Infinity, from.y + r, to.y + r),
      };
    }
    if (!erase && !painted.current) {
      painted.current = true;
      onPaintedChange(true);
    }
  };

  // After an eraser stroke that removed all the paint, the mask counts as empty again.
  const endStroke = () => {
    const wasStroke = !!last.current;
    last.current = null;
    if (wasStroke && erase && painted.current && canvas.current && !hasPaint(canvas.current, paintBox.current)) {
      painted.current = false;
      onPaintedChange(false);
    }
  };

  return (
    <>
      <canvas
        ref={canvas}
        aria-label="Paint where the image may change"
        className={`absolute inset-0 h-full w-full touch-none opacity-55 ${active ? "cursor-none" : "pointer-events-none invisible"}`}
        onPointerDown={(e) => {
          if (!active || e.button !== 0) return;
          e.currentTarget.setPointerCapture(e.pointerId);
          const p = toImage(e);
          last.current = p;
          stroke(p, p);
        }}
        onPointerMove={(e) => {
          if (!active) return;
          const r = e.currentTarget.getBoundingClientRect();
          setCursor({ x: e.clientX - r.left, y: e.clientY - r.top });
          if (!last.current) return;
          const p = toImage(e);
          stroke(last.current, p);
          last.current = p;
        }}
        onPointerUp={endStroke}
        onPointerCancel={endStroke}
        onPointerLeave={() => {
          setCursor(null);
          endStroke();
        }}
      />
      {active && cursor && (
        <span
          aria-hidden
          className="pointer-events-none absolute rounded-full border-2 border-white shadow-[0_0_0_1px_rgba(0,0,0,0.5)]"
          style={{ left: cursor.x - brush / 2, top: cursor.y - brush / 2, width: brush, height: brush }}
        />
      )}
    </>
  );
}
