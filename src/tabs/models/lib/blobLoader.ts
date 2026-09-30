// Image loader for the model details page: at most a few fetches at once, Blobs kept in RAM
// only while the page is open. Nothing is written to disk.
//  * `front` requests (the image the user just opened) jump ahead of the gallery thumbnails.
//  * `clear()` (page closed) drops every fetch that hasn't started, so they never reach Rust.

export class CancelledError extends Error {
  constructor() {
    super("cancelled");
    this.name = "CancelledError";
  }
}

type Waiter = { url: string; start: () => void; cancel: () => void };

export function createBlobLoader(fetchBlob: (url: string) => Promise<Blob>, maxParallel: number) {
  let running = 0;
  let waiting: Waiter[] = [];
  const blobs = new Map<string, Promise<Blob>>();

  // A finishing job hands its slot straight to the next waiter (running stays the same),
  // so a new call in the same tick can't slip in past the cap.
  function release() {
    const next = waiting.shift();
    if (next) next.start();
    else running--;
  }

  async function run(url: string, front: boolean): Promise<Blob> {
    if (running >= maxParallel) {
      await new Promise<void>((resolve, reject) => {
        const w: Waiter = { url, start: resolve, cancel: () => reject(new CancelledError()) };
        if (front) waiting.unshift(w);
        else waiting.push(w);
      });
    } else running++;
    try {
      return await fetchBlob(url);
    } finally {
      release();
    }
  }

  function load(url: string, opts: { front?: boolean } = {}): Promise<Blob> {
    const front = !!opts.front;
    let p = blobs.get(url);
    if (p) {
      // Already queued behind others: move it to the front.
      if (front) {
        const i = waiting.findIndex((w) => w.url === url);
        if (i > 0) waiting.unshift(...waiting.splice(i, 1));
      }
      return p;
    }
    p = run(url, front);
    const mine = p;
    p.catch(() => {
      if (blobs.get(url) === mine) blobs.delete(url);
    });
    blobs.set(url, p);
    return p;
  }

  /** Forget every Blob and drop the fetches that haven't started. */
  function clear() {
    const dropped = waiting;
    waiting = [];
    blobs.clear();
    for (const w of dropped) w.cancel();
  }

  return { load, clear, stats: () => ({ running, waiting: waiting.length, cached: blobs.size }) };
}
