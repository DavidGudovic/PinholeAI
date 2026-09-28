import { useLayoutEffect, useState, type RefObject } from "react";

/** Largest w×h with the image's aspect ratio that fits inside the container (never upscales past 2×). */
export function useFitBox(container: RefObject<HTMLElement | null>, naturalW: number, naturalH: number): { width: number; height: number } {
  const [box, setBox] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const el = container.current;
    if (!el || !naturalW || !naturalH) return;
    const measure = () => {
      const cs = getComputedStyle(el);
      const cw = el.clientWidth - (parseFloat(cs.paddingLeft) || 0) - (parseFloat(cs.paddingRight) || 0);
      const ch = el.clientHeight - (parseFloat(cs.paddingTop) || 0) - (parseFloat(cs.paddingBottom) || 0);
      const scale = Math.min(cw / naturalW, ch / naturalH, 2);
      const width = Math.max(1, Math.floor(naturalW * scale));
      const height = Math.max(1, Math.floor(naturalH * scale));
      setBox((b) => (b.width === width && b.height === height ? b : { width, height }));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [container, naturalW, naturalH]);
  return box;
}
