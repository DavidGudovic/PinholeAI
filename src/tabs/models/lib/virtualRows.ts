// Row math for the Browse grid, which keeps only the rows near the screen in the page
// (see ../VirtualGrid.tsx). Pure functions, so they can be tested without a browser.

/** Columns of `repeat(auto-fill, minmax(min, 1fr))` with this gap in a box this wide. */
export function columnsFor(width: number, min: number, gap: number): number {
  return Math.max(1, Math.floor((width + gap) / (min + gap)));
}

/**
 * Top of each row and the total height. `heights[i]` is row i's measured height, or
 * undefined when it hasn't been on screen yet (then `estimate` is used).
 * Returns `rows + 1` offsets: `tops[rows]` is the end of the last row plus one gap.
 */
export function rowTops(rows: number, heights: readonly (number | undefined)[], estimate: number, gap: number): number[] {
  const tops = new Array<number>(rows + 1);
  let y = 0;
  for (let i = 0; i < rows; i++) {
    tops[i] = y;
    y += (heights[i] ?? estimate) + gap;
  }
  tops[rows] = y;
  return tops;
}

/** First index whose value is greater than `y` (tops are increasing). */
function firstAbove(tops: readonly number[], y: number, rows: number): number {
  let lo = 0;
  let hi = rows;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (tops[mid] > y) hi = mid;
    else lo = mid + 1;
  }
  return lo;
}

/**
 * Rows [start, end) that touch the band from `from` to `to` (pixels from the top of the
 * list; the band already includes the margin kept around the screen).
 */
export function rowRange(tops: readonly number[], rows: number, from: number, to: number): [number, number] {
  if (rows === 0 || to < 0) return [0, Math.min(rows, 1)];
  const start = Math.max(0, firstAbove(tops, from, rows) - 1);
  const end = Math.max(start + 1, Math.min(rows, firstAbove(tops, to, rows)));
  return [start, end];
}
