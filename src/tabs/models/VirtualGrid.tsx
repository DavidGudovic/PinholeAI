// The Browse card grid. Only the rows on screen and up to OVERSCAN pixels around it are in
// the page; the rest is empty space of the same height. With hundreds of cards loaded,
// scrolling, filter changes and opening a model then cost about the same as with a few rows.
// Looks the same as `grid-template-columns: repeat(auto-fill, minmax(240px, 1fr))` with a
// 16px gap. Until the width is known (first render, tests) the first cards are in one plain grid.
import { Fragment, useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { columnsFor, rowRange, rowTops } from "./lib/virtualRows";

const MIN_COLUMN = 240;
const GAP = 16;
/** Rows are kept this far above and below the screen. */
export const OVERSCAN = 1200;
/** Cards shown before the width is known. */
const FIRST_CARDS = 48;

/** The row at the top of the screen and where it was, so it can be kept there. */
type Anchor = { el: HTMLElement; top: number; scrollTop: number };

export function VirtualGrid<T>({
  items,
  itemKey,
  renderItem,
  scrollRoot,
  after,
}: {
  items: readonly T[];
  itemKey: (item: T) => string | number;
  renderItem: (item: T) => ReactNode;
  /** The element that scrolls the grid (null: the window). */
  scrollRoot: Element | null;
  /** Placeholder cards shown after the last card, in the same columns. */
  after?: ReactNode;
}) {
  const list = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const cols = width > 0 ? columnsFor(width, MIN_COLUMN, GAP) : 0;
  const rows = cols ? Math.ceil(items.length / cols) : 0;

  // Measured row heights, forgotten when the columns change or another list is shown.
  const heights = useRef<(number | undefined)[]>([]);
  const layoutKey = `${cols}:${items.length ? itemKey(items[0]) : ""}`;
  const shownLayout = useRef(layoutKey);
  if (shownLayout.current !== layoutKey) {
    shownLayout.current = layoutKey;
    heights.current = [];
  }
  const [, setMeasured] = useState(0);
  const known = heights.current.filter((h): h is number => h != null);
  const colWidth = cols ? (width - GAP * (cols - 1)) / cols : MIN_COLUMN;
  // Before any row is measured: picture (4:5) plus the usual text and button.
  const estimate = known.length ? known.reduce((a, b) => a + b, 0) / known.length : colWidth * 1.25 + 190;
  const tops = rowTops(rows, heights.current, estimate, GAP);

  const [range, setRange] = useState<[number, number]>([0, 0]);
  const latest = useRef({ range, tops, rows });
  latest.current = { range, tops, rows };

  const scrollTopOf = useCallback(() => (scrollRoot ? scrollRoot.scrollTop : window.scrollY), [scrollRoot]);
  const rootTop = useCallback(() => (scrollRoot ? scrollRoot.getBoundingClientRect().top : 0), [scrollRoot]);
  /** The Models tab is hidden (another tab is open): nothing to measure. */
  const hidden = useCallback(() => (scrollRoot ? scrollRoot.clientHeight : window.innerHeight) === 0, [scrollRoot]);

  /** The screen, in pixels from the top of the list (null while the tab is hidden). */
  const viewport = useCallback((): [number, number] | null => {
    const el = list.current;
    if (!el || hidden()) return null;
    const listTop = el.getBoundingClientRect().top;
    const top = rootTop();
    const height = scrollRoot ? scrollRoot.clientHeight : window.innerHeight;
    return [top - listTop, top - listTop + height];
  }, [scrollRoot, rootTop, hidden]);

  // Keeping the rows on screen still. The browser's own scroll anchoring is off for the list
  // (WebKitGTK has none): when rows above the screen change height, or are swapped for
  // padding of a slightly different height, the scroll position is moved by the difference.
  const anchor = useRef<Anchor | null>(null);
  const setAnchor = useCallback(() => {
    const el = list.current;
    if (!el || hidden()) return;
    anchor.current = null;
    const top = rootTop();
    for (const row of Array.from(el.children) as HTMLElement[]) {
      const r = row.getBoundingClientRect();
      if (r.bottom > top) {
        anchor.current = { el: row, top: r.top, scrollTop: scrollTopOf() };
        return;
      }
    }
  }, [rootTop, scrollTopOf, hidden]);
  const keepAnchor = useCallback(() => {
    const a = anchor.current;
    if (!a || !a.el.isConnected || hidden()) return;
    // Where the row would be if only the user's own scrolling had moved it.
    const expected = a.top - (scrollTopOf() - a.scrollTop);
    const drift = a.el.getBoundingClientRect().top - expected;
    if (Math.abs(drift) >= 0.5) {
      if (scrollRoot) scrollRoot.scrollTop += drift;
      else window.scrollBy(0, drift);
    }
    setAnchor();
  }, [scrollRoot, scrollTopOf, setAnchor, hidden]);

  const update = useCallback(() => {
    const v = viewport();
    if (!v) return;
    const { tops, rows, range } = latest.current;
    const next = rowRange(tops, rows, v[0] - OVERSCAN, v[1] + OVERSCAN);
    if (next[0] !== range[0] || next[1] !== range[1]) setRange(next);
  }, [viewport]);

  // Width → columns. A hidden tab reports 0: keep the last width then.
  useLayoutEffect(() => {
    const el = list.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const measure = () => {
      if (el.clientWidth > 0) setWidth(el.clientWidth);
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Scrolling, a resized window or content above the list growing → rows to show, once per frame.
  useEffect(() => {
    if (!cols) return;
    const target: Element | Window = scrollRoot ?? window;
    let frame = 0;
    const onChange = () => {
      if (!frame)
        frame = requestAnimationFrame(() => {
          frame = 0;
          setAnchor();
          update();
        });
    };
    target.addEventListener("scroll", onChange, { passive: true });
    window.addEventListener("resize", onChange);
    const content = scrollRoot?.firstElementChild;
    const ro = content && typeof ResizeObserver !== "undefined" ? new ResizeObserver(onChange) : null;
    if (content) ro?.observe(content);
    return () => {
      target.removeEventListener("scroll", onChange);
      window.removeEventListener("resize", onChange);
      ro?.disconnect();
      cancelAnimationFrame(frame);
    };
  }, [cols, scrollRoot, update, setAnchor]);

  // After every change to the rows: keep the screen still, then pick the rows again.
  useLayoutEffect(() => {
    if (!cols) return;
    keepAnchor();
    update();
  });

  // Row heights.
  const rowObserver = useRef<ResizeObserver | null>(null);
  const live = useRef(keepAnchor);
  live.current = keepAnchor;
  const observeRow = useCallback((el: HTMLDivElement) => {
    if (typeof ResizeObserver === "undefined") return;
    rowObserver.current ??= new ResizeObserver((entries) => {
      let changed = false;
      for (const e of entries) {
        const el = e.target as HTMLElement;
        if (!el.isConnected || el.dataset.layout !== shownLayout.current) continue;
        const i = Number(el.dataset.row);
        const h = el.getBoundingClientRect().height;
        const old = heights.current[i];
        if (old != null && Math.abs(old - h) < 0.5) continue;
        heights.current[i] = h;
        changed = true;
      }
      // Runs after layout, before painting: a row above that grew is corrected right away.
      live.current();
      if (changed) setMeasured((m) => m + 1);
    });
    const ro = rowObserver.current;
    ro.observe(el);
    // Rows that leave the page are no longer watched (and can be freed).
    return () => ro.unobserve(el);
  }, []);
  useEffect(
    () => () => {
      rowObserver.current?.disconnect();
      rowObserver.current = null;
    },
    [],
  );

  const columns = { gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` };
  const [start, end] = [Math.min(range[0], rows), Math.min(range[1], rows)];
  const shown: ReactNode[] = [];
  for (let r = start; r < end; r++) {
    const rowItems = items.slice(r * cols, r * cols + cols);
    const last = r === rows - 1;
    shown.push(
      <div key={`${layoutKey}:${r}`} ref={observeRow} data-row={r} data-layout={layoutKey} className="grid gap-4" style={columns}>
        {rowItems.map((it) => (
          <Fragment key={itemKey(it)}>{renderItem(it)}</Fragment>
        ))}
        {/* Placeholders continue the last row, as in one grid. */}
        {last && after}
      </div>,
    );
  }

  return (
    <>
      {/* overflow-anchor: none: the scroll position is kept by the code above, not by the browser. */}
      <div
        ref={list}
        className={cols ? "flex flex-col gap-4" : "grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-4"}
        style={cols ? { paddingTop: tops[start], paddingBottom: tops[rows] - tops[end], overflowAnchor: "none" } : undefined}
      >
        {cols
          ? shown
          : items.slice(0, FIRST_CARDS).map((it) => (
              <Fragment key={itemKey(it)}>{renderItem(it)}</Fragment>
            ))}
        {!cols && after}
      </div>
      {cols > 0 && rows === 0 && after && (
        <div className="grid gap-4" style={columns}>
          {after}
        </div>
      )}
    </>
  );
}
