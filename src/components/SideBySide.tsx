// Two pictures shown whole, next to each other (or one above the other when that shows them
// bigger), e.g. the reference picture and the result made with it. Both stay blob URLs in memory.
import { useLayoutEffect, useRef, useState } from "react";

export interface SidePicture {
  url: string;
  width: number;
  height: number;
  label: string;
}

/** Space between the two pictures, in px. */
const GAP = 12;

export interface SideLayout {
  direction: "row" | "column";
  first: { width: number; height: number };
  second: { width: number; height: number };
}

/**
 * Sizes for both pictures inside `w`×`h`: same height in a row, or same width in a column,
 * whichever shows more of them. Neither is drawn larger than 2× its own size.
 */
export function sideLayout(
  w: number,
  h: number,
  a: { width: number; height: number },
  b: { width: number; height: number },
): SideLayout {
  const ra = a.width / a.height;
  const rb = b.width / b.height;
  // Row: shared height.
  const rowH = Math.max(1, Math.min(h, (w - GAP) / (ra + rb), 2 * Math.min(a.height, b.height)));
  // Column: shared width.
  const colW = Math.max(1, Math.min(w, (h - GAP) / (1 / ra + 1 / rb), 2 * Math.min(a.width, b.width)));
  const rowArea = rowH * rowH * (ra + rb);
  const colArea = colW * colW * (1 / ra + 1 / rb);
  const size = (width: number, height: number) => ({ width: Math.floor(width), height: Math.floor(height) });
  return rowArea >= colArea
    ? { direction: "row", first: size(rowH * ra, rowH), second: size(rowH * rb, rowH) }
    : { direction: "column", first: size(colW, colW / ra), second: size(colW, colW / rb) };
}

export function SideBySide({ first, second }: { first: SidePicture; second: SidePicture }) {
  const box = useRef<HTMLDivElement>(null);
  const [space, setSpace] = useState({ w: 0, h: 0 });
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const measure = () => setSpace((s) => (s.w === el.clientWidth && s.h === el.clientHeight ? s : { w: el.clientWidth, h: el.clientHeight }));
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const layout = space.w > 0 && space.h > 0 ? sideLayout(space.w, space.h, first, second) : null;

  return (
    <div ref={box} data-testid="side-by-side" className="flex h-full min-h-0 w-full min-w-0 items-center justify-center">
      {layout && (
        <div
          role="group"
          aria-label={`${first.label} and ${second.label} side by side`}
          className="flex items-center justify-center"
          style={{ flexDirection: layout.direction, gap: GAP }}
        >
          {[
            { p: first, s: layout.first },
            { p: second, s: layout.second },
          ].map(({ p, s }, i) => (
            <div key={i} className="relative shrink-0 overflow-hidden rounded-lg shadow-lg ring-1 ring-black/5 dark:ring-white/10" style={{ width: s.width, height: s.height }}>
              <img src={p.url} alt={p.label} className="h-full w-full" draggable={false} />
              <span className="pointer-events-none absolute top-2 left-2 rounded-md bg-black/55 px-1.5 py-0.5 text-[11px] font-medium text-white">{p.label}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
