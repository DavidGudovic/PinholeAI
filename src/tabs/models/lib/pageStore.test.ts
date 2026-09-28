import { describe, expect, it, vi } from "vitest";
import type { BrowsePage } from "../../../lib/types";
import { PageStore } from "./pageStore";

const page = (ids: number[], nextCursor: string | null, extra: Partial<BrowsePage> = {}): BrowsePage =>
  ({
    items: ids.map((versionId) => ({ versionId })),
    nextCursor,
    offline: false,
    partial: false,
    checked: ids.length,
    hiddenByContent: 0,
    hiddenByFilters: 0,
    ...extra,
  }) as BrowsePage;

describe("PageStore", () => {
  it("shares one request per page and caches the answer", async () => {
    const store = new PageStore();
    const fetcher = vi.fn(async () => page([1, 2], "c1"));
    const [a, b] = await Promise.all([store.load("k", null, fetcher), store.load("k", null, fetcher)]);
    expect(a).toBe(b);
    expect(await store.load("k", null, fetcher)).toBe(a);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(store.has("k", null)).toBe(true);
    expect(store.has("other", null)).toBe(false);
  });

  it("doesn't cache errors or offline placeholders", async () => {
    const store = new PageStore();
    await expect(store.load("k", null, () => Promise.reject(new Error("net")))).rejects.toThrow("net");
    expect(store.has("k", null)).toBe(false);
    await store.load("k", null, async () => page([], null, { offline: true }));
    expect(store.get("k", null)).toBeNull();
  });

  it("rebuilds the chain of pages in order and stops at a gap", () => {
    const store = new PageStore();
    store.put("k", null, page([1], "c1"));
    store.put("k", "c1", page([2], "c2", { partial: true }));
    store.put("k", "c3", page([4], null));
    const chain = store.chain("k")!;
    expect(chain.pages.map((p) => p.items[0].versionId)).toEqual([1, 2]);
    expect(chain.nextCursor).toBe("c2");
    expect(chain.partial).toBe(true);
    expect(store.chain("nothing")).toBeNull();
  });

  it("survives a cursor loop", () => {
    const store = new PageStore();
    store.put("k", null, page([1], "c1"));
    store.put("k", "c1", page([2], "c1"));
    expect(store.chain("k")!.pages).toHaveLength(2);
  });

  it("forgets the oldest pages and expired ones", () => {
    let now = 0;
    const store = new PageStore(2, 1000, () => now);
    store.put("a", null, page([1], null));
    store.put("b", null, page([2], null));
    store.get("a", null); // a is now the most recent
    store.put("c", null, page([3], null));
    expect(store.get("b", null)).toBeNull();
    expect(store.get("a", null)).not.toBeNull();
    now = 5000;
    expect(store.get("a", null)).toBeNull();
    store.clear();
    expect(store.size).toBe(0);
  });
});
