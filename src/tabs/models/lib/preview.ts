// CivitAI preview images: bytes come from Rust (`fetch_preview`), shown as Blob URLs.
// The WebView never loads a remote URL. Fetches start only when the card is near the
// viewport, run at most 4 at a time, and Blob URLs are revoked on unmount. A small
// in-memory cache (RAM only) avoids fetching the same preview twice when filters change.
import { useEffect, useRef, useState, type RefObject } from "react";
import { fetchPreview } from "../../../lib/api";
import { sniffImageType } from "./words";

const MAX_IN_FLIGHT = 4;
const CACHE_MAX = 160;
const cache = new Map<string, Blob>();

function cacheGet(url: string): Blob | null {
  const b = cache.get(url);
  if (!b) return null;
  cache.delete(url); // refresh LRU position
  cache.set(url, b);
  return b;
}

function cachePut(url: string, blob: Blob) {
  cache.set(url, blob);
  while (cache.size > CACHE_MAX) cache.delete(cache.keys().next().value!);
}
let inFlight = 0;
const waiting: (() => void)[] = [];

function next() {
  const run = waiting.shift();
  if (run) run();
}

/** Run `job` when a slot is free; skipped entirely if `alive()` turns false while queued. */
function schedule<T>(job: () => Promise<T>, alive: () => boolean): Promise<T | null> {
  return new Promise((resolve, reject) => {
    const run = () => {
      if (!alive()) {
        resolve(null);
        next();
        return;
      }
      inFlight += 1;
      job()
        .then(resolve, reject)
        .finally(() => {
          inFlight -= 1;
          next();
        });
    };
    if (inFlight < MAX_IN_FLIGHT) run();
    else waiting.push(run);
  });
}

function toBytes(buf: unknown): Uint8Array {
  if (buf instanceof ArrayBuffer) return new Uint8Array(buf);
  if (ArrayBuffer.isView(buf)) return new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
  if (Array.isArray(buf)) return Uint8Array.from(buf as number[]);
  return new Uint8Array();
}

/** True once the element has been near the viewport (stays true). */
export function useNearViewport<T extends Element>(ref: RefObject<T | null>, margin = "400px"): boolean {
  const [seen, setSeen] = useState(false);
  useEffect(() => {
    if (seen) return;
    const el = ref.current;
    if (!el || typeof IntersectionObserver === "undefined") {
      setSeen(true);
      return;
    }
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setSeen(true);
          io.disconnect();
        }
      },
      { rootMargin: margin },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [ref, margin, seen]);
  return seen;
}

/** Live visibility (false while an ancestor is `hidden` / display:none, e.g. an inactive tab). */
export function useIsVisible<T extends Element>(ref: RefObject<T | null>): boolean {
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const io = new IntersectionObserver((entries) => setVisible(entries.some((e) => e.isIntersecting)));
    io.observe(el);
    return () => io.disconnect();
  }, [ref]);
  return visible;
}

export type PreviewState = { src: string | null; failed: boolean; loading: boolean };

export function usePreviewBlob(url: string | null, enabled: boolean): PreviewState {
  const [state, setState] = useState<PreviewState>({ src: null, failed: false, loading: false });
  const objectUrl = useRef<string | null>(null);

  useEffect(() => {
    if (!url || !enabled) return;
    let alive = true;
    const show = (blob: Blob) => {
      const u = URL.createObjectURL(blob);
      objectUrl.current = u;
      setState({ src: u, failed: false, loading: false });
    };
    const cached = cacheGet(url);
    if (cached) show(cached);
    else {
      setState({ src: null, failed: false, loading: true });
      schedule(() => fetchPreview(url), () => alive)
        .then((buf) => {
          if (!alive || buf == null) return;
          const bytes = toBytes(buf);
          if (!bytes.length) throw new Error("empty");
          const blob = new Blob([bytes as BlobPart], { type: sniffImageType(bytes) });
          cachePut(url, blob);
          show(blob);
        })
        .catch(() => {
          if (alive) setState({ src: null, failed: true, loading: false });
        });
    }
    return () => {
      alive = false;
      if (objectUrl.current) {
        URL.revokeObjectURL(objectUrl.current);
        objectUrl.current = null;
      }
    };
  }, [url, enabled]);

  return state;
}
