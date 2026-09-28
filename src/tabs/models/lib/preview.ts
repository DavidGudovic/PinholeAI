// CivitAI preview images: bytes come from Rust (`fetch_preview`), shown as Blob URLs.
// The WebView never loads a remote URL.
//  * Cards report whether they are on screen, near it, or away (two shared observers).
//  * Fetches run 8 at a time, on-screen cards first; a queued fetch is dropped when its
//    card scrolls away or unmounts (see previewQueue.ts).
//  * Previews stay in a RAM-only LRU (≈48 MB) for the whole session, so scrolling back,
//    switching filters or tabs doesn't fetch them again. Nothing is written to disk.
//  * A card far off screen gives its Blob URL back, so memory stays bounded by the LRU.
import { useEffect, useRef, useState, type RefObject } from "react";
import { fetchPreview } from "../../../lib/api";
import { BlobLru, PreviewScheduler, type PreviewHandle, type Priority } from "./previewQueue";
import { measureSince } from "./perf";
import { sniffImageType } from "./words";

const MAX_IN_FLIGHT = 8;
const CACHE_BYTES = 48 * 1024 * 1024;
const CACHE_COUNT = 600;
/** Start fetching this far before a card scrolls into view. */
export const NEAR_MARGIN = "1200px 0px";

const cache = new BlobLru<Blob>(CACHE_BYTES, CACHE_COUNT);

function toBytes(buf: unknown): Uint8Array {
  if (buf instanceof ArrayBuffer) return new Uint8Array(buf);
  if (ArrayBuffer.isView(buf)) return new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
  if (Array.isArray(buf)) return Uint8Array.from(buf as number[]);
  return new Uint8Array();
}

const scheduler = new PreviewScheduler<Blob>(async (url) => {
  const cached = cache.get(url);
  if (cached) return cached;
  const started = performance.now();
  const bytes = toBytes(await fetchPreview(url));
  if (!bytes.length) throw new Error("empty preview");
  const blob = new Blob([bytes as BlobPart], { type: sniffImageType(bytes) });
  cache.set(url, blob);
  measureSince("pinhole:preview", started);
  return blob;
}, MAX_IN_FLIGHT);

// ---------------------------------------------------------------- viewport tracking
export type Visibility = "visible" | "near" | "away";

type Watcher = { visible: boolean; near: boolean; cb: (v: Visibility) => void };
const watchers = new Map<Element, Watcher>();
let visibleObserver: IntersectionObserver | null = null;
let nearObserver: IntersectionObserver | null = null;

function report(w: Watcher) {
  w.cb(w.visible ? "visible" : w.near ? "near" : "away");
}

function observers(): [IntersectionObserver, IntersectionObserver] | null {
  if (typeof IntersectionObserver === "undefined") return null;
  if (!visibleObserver || !nearObserver) {
    visibleObserver = new IntersectionObserver((entries) => {
      for (const e of entries) {
        const w = watchers.get(e.target);
        if (w && w.visible !== e.isIntersecting) {
          w.visible = e.isIntersecting;
          report(w);
        }
      }
    });
    nearObserver = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          const w = watchers.get(e.target);
          if (w && w.near !== e.isIntersecting) {
            w.near = e.isIntersecting;
            report(w);
          }
        }
      },
      { rootMargin: NEAR_MARGIN },
    );
  }
  return [visibleObserver, nearObserver];
}

/** Where a card is relative to the screen (two observers shared by every card). */
export function useVisibility<T extends Element>(ref: RefObject<T | null>): Visibility {
  const [vis, setVis] = useState<Visibility>("away");
  useEffect(() => {
    const el = ref.current;
    const obs = observers();
    if (!el || !obs) {
      setVis("visible");
      return;
    }
    watchers.set(el, { visible: false, near: false, cb: setVis });
    obs[0].observe(el);
    obs[1].observe(el);
    return () => {
      obs[0].unobserve(el);
      obs[1].unobserve(el);
      watchers.delete(el);
    };
  }, [ref]);
  return vis;
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

// ---------------------------------------------------------------- preview hook
export type PreviewState = { src: string | null; failed: boolean; loading: boolean };

const EMPTY: PreviewState = { src: null, failed: false, loading: false };

/**
 * Blob URL for a preview. Fetches while the card is on screen or near it (on-screen first);
 * a card that scrolls away drops its queued fetch and gives its Blob URL back.
 */
export function usePreviewBlob(url: string | null, visibility: Visibility): PreviewState {
  const [state, setState] = useState<PreviewState>(EMPTY);
  const objectUrl = useRef<string | null>(null);
  const handle = useRef<PreviewHandle<Blob> | null>(null);
  const failedUrl = useRef<string | null>(null);
  const active = visibility !== "away";
  const priority: Priority = visibility === "visible" ? 0 : 1;

  // Load (or reuse) while on/near screen; give everything back when away.
  useEffect(() => {
    if (!url || !active || failedUrl.current === url) return;
    let alive = true;
    const show = (blob: Blob) => {
      if (!alive) return;
      objectUrl.current = URL.createObjectURL(blob);
      setState({ src: objectUrl.current, failed: false, loading: false });
    };
    const cached = cache.get(url);
    if (cached) show(cached);
    else {
      setState({ src: null, failed: false, loading: true });
      const h = scheduler.request(url, priority);
      handle.current = h;
      h.promise
        .then((blob) => {
          if (blob) show(blob);
        })
        .catch(() => {
          failedUrl.current = url;
          if (alive) setState({ src: null, failed: true, loading: false });
        });
    }
    return () => {
      alive = false;
      handle.current?.release();
      handle.current = null;
      if (objectUrl.current) {
        URL.revokeObjectURL(objectUrl.current);
        objectUrl.current = null;
      }
      setState((s) => (s.failed ? s : EMPTY));
    };
    // Priority changes are applied below without restarting the load.
  }, [url, active]);

  useEffect(() => {
    handle.current?.setPriority(priority);
  }, [priority]);

  return state;
}

/** For tests / diagnostics. */
export const previewStats = () => ({ inFlight: scheduler.inFlight, queued: scheduler.queued, cachedBytes: cache.bytes, cached: cache.size });
