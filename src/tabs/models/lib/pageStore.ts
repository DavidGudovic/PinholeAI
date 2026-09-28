// Recent Browse pages, kept in RAM only (never in storage), so going back to filters you
// just used shows the grid instantly, and the next page can be fetched ahead of the scroll.
// Pages are keyed by the filters key + cursor; one request per key is shared by everyone
// who asks while it runs (prefetch and scroll ask for the same page).
import type { BrowsePage } from "../../../lib/types";

interface Entry {
  page: BrowsePage;
  at: number;
}

export interface Chain {
  pages: BrowsePage[];
  /** Cursor of the page after the last cached one (null = end, or not cached yet). */
  nextCursor: string | null;
  /** The last cached page stopped early ("Load more"). */
  partial: boolean;
}

export class PageStore {
  private entries = new Map<string, Entry>();
  private inflight = new Map<string, Promise<BrowsePage>>();

  constructor(
    private readonly maxPages = 80,
    private readonly ttlMs = 10 * 60_000,
    private readonly now: () => number = () => Date.now(),
  ) {}

  static key(filtersKey: string, cursor: string | null): string {
    return `${filtersKey}\u0000${cursor ?? ""}`;
  }

  /** A fresh cached page, or null. */
  get(filtersKey: string, cursor: string | null): BrowsePage | null {
    const k = PageStore.key(filtersKey, cursor);
    const e = this.entries.get(k);
    if (!e) return null;
    if (this.now() - e.at > this.ttlMs) {
      this.entries.delete(k);
      return null;
    }
    // Refresh LRU position.
    this.entries.delete(k);
    this.entries.set(k, e);
    return e.page;
  }

  has(filtersKey: string, cursor: string | null): boolean {
    return this.get(filtersKey, cursor) !== null || this.inflight.has(PageStore.key(filtersKey, cursor));
  }

  put(filtersKey: string, cursor: string | null, page: BrowsePage) {
    // Offline pages are placeholders, not results.
    if (page.offline) return;
    const k = PageStore.key(filtersKey, cursor);
    this.entries.delete(k);
    this.entries.set(k, { page, at: this.now() });
    while (this.entries.size > this.maxPages) this.entries.delete(this.entries.keys().next().value!);
  }

  /** The cached page, or one shared request for it (results are cached, errors are not). */
  load(filtersKey: string, cursor: string | null, fetcher: () => Promise<BrowsePage>): Promise<BrowsePage> {
    const cached = this.get(filtersKey, cursor);
    if (cached) return Promise.resolve(cached);
    const k = PageStore.key(filtersKey, cursor);
    let p = this.inflight.get(k);
    if (!p) {
      p = fetcher()
        .then((page) => {
          this.put(filtersKey, cursor, page);
          return page;
        })
        .finally(() => this.inflight.delete(k));
      this.inflight.set(k, p);
    }
    return p;
  }

  /** Every cached page of these filters in order, starting from the first page. */
  chain(filtersKey: string): Chain | null {
    const pages: BrowsePage[] = [];
    let cursor: string | null = null;
    const seen = new Set<string>();
    for (;;) {
      const page = this.get(filtersKey, cursor);
      if (!page) break;
      pages.push(page);
      if (!page.nextCursor || seen.has(page.nextCursor)) break;
      seen.add(page.nextCursor);
      cursor = page.nextCursor;
    }
    if (!pages.length) return null;
    const last = pages[pages.length - 1];
    return { pages, nextCursor: last.nextCursor, partial: last.partial };
  }

  clear() {
    this.entries.clear();
  }

  get size(): number {
    return this.entries.size;
  }
}
