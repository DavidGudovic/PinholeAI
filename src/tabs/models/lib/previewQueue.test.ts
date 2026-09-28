import { describe, expect, it } from "vitest";
import { BlobLru, PreviewScheduler } from "./previewQueue";

/** A fetch function whose calls the test resolves by hand. */
function manual() {
  const calls: { url: string; resolve: (v: string) => void; reject: (e: unknown) => void }[] = [];
  const fetchFn = (url: string) => new Promise<string>((resolve, reject) => calls.push({ url, resolve, reject }));
  return { calls, fetchFn };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("BlobLru", () => {
  const v = (size: number) => ({ size });
  it("evicts the least recently used by count and by bytes", () => {
    const lru = new BlobLru<{ size: number }>(100, 2);
    lru.set("a", v(10));
    lru.set("b", v(10));
    lru.get("a");
    lru.set("c", v(10));
    expect(lru.get("b")).toBeUndefined();
    expect(lru.get("a")).toBeDefined();
    lru.set("d", v(95));
    expect(lru.size).toBe(1);
    expect(lru.bytes).toBe(95);
  });
  it("never keeps one value bigger than the whole cache", () => {
    const lru = new BlobLru<{ size: number }>(100, 10);
    lru.set("a", v(10));
    lru.set("huge", v(500));
    expect(lru.get("huge")).toBeUndefined();
    expect(lru.bytes).toBe(10);
  });
});

describe("PreviewScheduler", () => {
  it("runs at most N at a time, on-screen cards first", async () => {
    const { calls, fetchFn } = manual();
    const s = new PreviewScheduler(fetchFn, 2);
    s.request("near-1", 1);
    s.request("near-2", 1);
    s.request("near-3", 1);
    s.request("visible", 0);
    expect(calls.map((c) => c.url)).toEqual(["near-1", "near-2"]);
    expect(s.queued).toBe(2);
    calls[0].resolve("x");
    await tick();
    expect(calls.map((c) => c.url)).toEqual(["near-1", "near-2", "visible"]);
    expect(s.inFlight).toBe(2);
  });

  it("drops a queued fetch once every card that wanted it scrolled away", async () => {
    const { calls, fetchFn } = manual();
    const s = new PreviewScheduler(fetchFn, 1);
    s.request("busy", 0);
    const a = s.request("gone", 1);
    const b = s.request("gone", 1);
    const kept = s.request("kept", 1);
    a.release();
    expect(s.queued).toBe(2);
    b.release();
    expect(s.queued).toBe(1);
    await expect(a.promise).resolves.toBeNull();
    calls[0].resolve("x");
    await tick();
    expect(calls.map((c) => c.url)).toEqual(["busy", "kept"]);
    calls[1].resolve("k");
    await expect(kept.promise).resolves.toBe("k");
  });

  it("shares one fetch per URL and raises its priority when a card scrolls into view", async () => {
    const { calls, fetchFn } = manual();
    const s = new PreviewScheduler(fetchFn, 1);
    s.request("busy", 0);
    s.request("a", 1);
    const b = s.request("b", 1);
    const b2 = s.request("b", 1);
    b.setPriority(0);
    calls[0].resolve("x");
    await tick();
    expect(calls.map((c) => c.url)).toEqual(["busy", "b"]);
    calls[1].resolve("B");
    await expect(b2.promise).resolves.toBe("B");
  });

  it("a running fetch finishes even when its card leaves (the result is cached)", async () => {
    const { calls, fetchFn } = manual();
    const s = new PreviewScheduler(fetchFn, 1);
    const h = s.request("a", 0);
    h.release();
    calls[0].reject(new Error("boom"));
    await expect(h.promise).rejects.toThrow("boom");
    await tick();
    expect(s.inFlight).toBe(0);
  });
});
