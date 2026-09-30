// Before/after comparison slider. Drag anywhere, or focus the handle and use ←/→.
import { useRef, useState, type PointerEvent } from "react";
import { MoveHorizontal } from "lucide-react";
import { cx, focusRing } from "../../components/ui";
import type { ImgRef } from "../../lib/state/model";

export function CompareView({ before, after, width, height, beforeLabel = "Before", afterLabel = "After" }: { before: ImgRef; after: ImgRef; width: number; height: number; beforeLabel?: string; afterLabel?: string }) {
  const [pos, setPos] = useState(50);
  const dragging = useRef(false);
  const box = useRef<HTMLDivElement>(null);

  const setFromEvent = (e: PointerEvent) => {
    const r = box.current?.getBoundingClientRect();
    if (!r) return;
    setPos(Math.min(100, Math.max(0, ((e.clientX - r.left) / r.width) * 100)));
  };

  return (
    <div
      ref={box}
      className="relative cursor-ew-resize touch-none overflow-hidden rounded-lg shadow-lg ring-1 ring-black/5 select-none dark:ring-white/10"
      style={{ width, height }}
      onPointerDown={(e) => {
        dragging.current = true;
        e.currentTarget.setPointerCapture(e.pointerId);
        setFromEvent(e);
      }}
      // No button held means the drag ended somewhere we didn't hear about.
      onPointerMove={(e) => dragging.current && e.buttons !== 0 && setFromEvent(e)}
      onPointerUp={() => {
        dragging.current = false;
      }}
      // Pen/touch or the OS can take the pointer away: stop dragging then too.
      onPointerCancel={() => {
        dragging.current = false;
      }}
      onLostPointerCapture={() => {
        dragging.current = false;
      }}
    >
      <img src={before.url} alt={beforeLabel} className="absolute inset-0 h-full w-full" draggable={false} />
      <img src={after.url} alt={afterLabel} className="absolute inset-0 h-full w-full" style={{ clipPath: `inset(0 0 0 ${pos}%)` }} draggable={false} />
      <span className="pointer-events-none absolute top-2 left-2 rounded-md bg-black/55 px-1.5 py-0.5 text-[11px] font-medium text-white">{beforeLabel}</span>
      <span className="pointer-events-none absolute top-2 right-2 rounded-md bg-black/55 px-1.5 py-0.5 text-[11px] font-medium text-white">{afterLabel}</span>
      <div className="pointer-events-none absolute inset-y-0 w-0.5 -translate-x-1/2 bg-white shadow-[0_0_0_1px_rgba(0,0,0,0.25)]" style={{ left: `${pos}%` }} />
      <button
        type="button"
        role="slider"
        aria-label="Compare before and after"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(pos)}
        onKeyDown={(e) => {
          if (e.key === "ArrowLeft") setPos((p) => Math.max(0, p - 5));
          else if (e.key === "ArrowRight") setPos((p) => Math.min(100, p + 5));
          else if (e.key === "Home") setPos(0);
          else if (e.key === "End") setPos(100);
          else return;
          e.preventDefault();
        }}
        className={cx("absolute top-1/2 flex h-8 w-8 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full bg-white text-neutral-800 shadow-lg", focusRing)}
        style={{ left: `${pos}%` }}
      >
        <MoveHorizontal className="h-4 w-4" />
      </button>
    </div>
  );
}
