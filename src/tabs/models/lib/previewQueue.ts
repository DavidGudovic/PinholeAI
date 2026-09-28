// Pure parts of the Browse preview pipeline (no React, unit-tested):
//  * BlobLru — a RAM-only LRU capped by bytes and count (previews are never stored on disk);
//  * PreviewScheduler — runs at most N fetches at a time, cards on screen first, and drops a
//    queued fetch as soon as every card that wanted it has scrolled away or unmounted.

/** 0 = on screen, 1 = near the screen (about to scroll in). */
export type Priority = 0 | 1;

export class BlobLru<V extends { size: number }> {
  private map = new Map<string, V>();
  private total = 0;

  constructor(
    private readonly maxBytes: number,
    private readonly maxCount: number,
  ) {}

  get(key: string): V | undefined {
    const v = this.map.get(key);
    if (v === undefined) return undefined;
    this.map.delete(key);
    this.map.set(key, v);
    return v;
  }

  set(key: string, value: V) {
    const old = this.map.get(key);
    if (old !== undefined) {
      this.total -= old.size;
      this.map.delete(key);
    }
    // One oversized value would evict everything else: don't keep it.
    if (value.size > this.maxBytes) return;
    this.map.set(key, value);
    this.total += value.size;
    while (this.map.size > this.maxCount || this.total > this.maxBytes) {
      const oldest = this.map.keys().next().value!;
      this.total -= this.map.get(oldest)!.size;
      this.map.delete(oldest);
    }
  }

  clear() {
    this.map.clear();
    this.total = 0;
  }

  get bytes(): number {
    return this.total;
  }

  get size(): number {
    return this.map.size;
  }
}

/** One card's interest in one preview. */
export interface PreviewHandle<T> {
  /** Resolves with the result, or null when the fetch was dropped before it started. */
  readonly promise: Promise<T | null>;
  setPriority(p: Priority): void;
  /** The card scrolled away or unmounted. */
  release(): void;
}

interface Job<T> {
  url: string;
  seq: number;
  state: "queued" | "running";
  holders: Set<{ priority: Priority }>;
  promise: Promise<T | null>;
  resolve: (v: T | null) => void;
  reject: (e: unknown) => void;
}

export class PreviewScheduler<T> {
  private jobs = new Map<string, Job<T>>();
  private running = 0;
  private seq = 0;

  constructor(
    private readonly fetchFn: (url: string) => Promise<T>,
    private readonly maxInFlight = 8,
  ) {}

  request(url: string, priority: Priority): PreviewHandle<T> {
    let job = this.jobs.get(url);
    if (!job) {
      let resolve!: (v: T | null) => void;
      let reject!: (e: unknown) => void;
      const promise = new Promise<T | null>((res, rej) => {
        resolve = res;
        reject = rej;
      });
      job = { url, seq: this.seq++, state: "queued", holders: new Set(), promise, resolve, reject };
      this.jobs.set(url, job);
    }
    const holder = { priority };
    job.holders.add(holder);
    const j = job;
    let released = false;
    this.pump();
    return {
      promise: j.promise,
      setPriority: (p) => {
        if (released || holder.priority === p) return;
        holder.priority = p;
        this.pump();
      },
      release: () => {
        if (released) return;
        released = true;
        j.holders.delete(holder);
        if (j.state === "queued" && j.holders.size === 0 && this.jobs.get(j.url) === j) {
          this.jobs.delete(j.url);
          j.resolve(null);
        }
      },
    };
  }

  private priorityOf(job: Job<T>): number {
    let p = 2;
    for (const h of job.holders) p = Math.min(p, h.priority);
    return p;
  }

  private next(): Job<T> | null {
    let best: Job<T> | null = null;
    let bestP = 3;
    for (const job of this.jobs.values()) {
      if (job.state !== "queued") continue;
      const p = this.priorityOf(job);
      if (p < bestP || (p === bestP && best && job.seq < best.seq)) {
        best = job;
        bestP = p;
      }
    }
    return best;
  }

  private pump() {
    while (this.running < this.maxInFlight) {
      const job = this.next();
      if (!job) return;
      job.state = "running";
      this.running += 1;
      this.fetchFn(job.url)
        .then(job.resolve, job.reject)
        .finally(() => {
          this.running -= 1;
          if (this.jobs.get(job.url) === job) this.jobs.delete(job.url);
          this.pump();
        });
    }
  }

  /** Fetches running right now. */
  get inFlight(): number {
    return this.running;
  }

  /** Fetches waiting for a slot. */
  get queued(): number {
    let n = 0;
    for (const j of this.jobs.values()) if (j.state === "queued") n += 1;
    return n;
  }
}
