/// <reference types="vite/client" />
// Dev-only timings for the Browse page, recorded as Performance API measures (see the
// WebView's dev tools → Performance). Never logged, never sent anywhere; the names are
// fixed labels (no URLs, no search text). Production builds record nothing.
const DEV = import.meta.env.DEV;

/** Record `name` as a measure from `start` (a `performance.now()` value) to now. */
export function measureSince(name: string, start: number) {
  if (!DEV || typeof performance === "undefined" || typeof performance.measure !== "function") return;
  try {
    performance.measure(name, { start, end: performance.now() });
  } catch {
    /* older engines: skip */
  }
}
