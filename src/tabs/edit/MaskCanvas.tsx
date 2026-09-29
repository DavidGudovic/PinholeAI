// "Only change here" brush. Paints on a canvas at the image's real resolution
// (CSS-scaled over the image); exported as a white-on-black PNG of the same size.
import { useEffect, useImperativeHandle, useRef, useState, type PointerEvent, type Ref } from "react";

export interface MaskHandle {
  clear(): void;
  /** White = change. Null when nothing is painted. */
  exportPng(): Promise<Blob | null>;
}

const PAINT = "rgba(245, 158, 11, 1)"; // amber-500, shown at reduced opacity

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
  const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null);

  // New image size → start over.
  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    c.width = width;
    c.height = height;
    painted.current = false;
    onPaintedChange(false);
  }, [width, height]);

  useImperativeHandle(ref, () => ({
    clear() {
      const c = canvas.current;
      c?.getContext("2d")?.clearRect(0, 0, c.width, c.height);
      painted.current = false;
      onPaintedChange(false);
    },
    async exportPng() {
      const c = canvas.current;
      if (!c || !painted.current) return null;
      const tmp = document.createElement("canvas");
      tmp.width = c.width;
      tmp.height = c.height;
      const t = tmp.getContext("2d")!;
      t.drawImage(c, 0, 0);
      // Everything erased again: nothing painted (an all-black mask would change nothing).
      // Anti-aliased eraser edges can leave a few nearly transparent pixels.
      const px = t.getImageData(0, 0, tmp.width, tmp.height).data;
      let any = false;
      for (let i = 3; i < px.length; i += 4) {
        if (px[i] > 16) {
          any = true;
          break;
        }
      }
      if (!any) return null;
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
    if (!erase && !painted.current) {
      painted.current = true;
      onPaintedChange(true);
    }
  };

  return (
    <>
      <canvas
        ref={canvas}
        aria-label="Paint where the image may change"
        className={`absolute inset-0 h-full w-full touch-none opacity-55 ${active ? "cursor-none" : "pointer-events-none"}`}
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
        onPointerUp={() => {
          last.current = null;
        }}
        onPointerLeave={() => {
          setCursor(null);
          last.current = null;
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
