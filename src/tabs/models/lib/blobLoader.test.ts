import { describe, expect, it } from "vitest";
import { CancelledError, createBlobLoader } from "./blobLoader";

/** A fetch whose calls the test resolves by hand. */
function manual() {
  const calls: { url: string; resolve: () => void }[] = [];
  const fetchBlob = (url: string) => new Promise<Blob>((resolve) => calls.push({ url, resolve: () => resolve(new Blob([url])) }));
  return { calls, fetchBlob };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("details page image loader", () => {
  it("fetches the opened image before the waiting thumbnails", async () => {
    const { calls, fetchBlob } = manual();
    const loader = createBlobLoader(fetchBlob, 4);
    for (let i = 0; i < 6; i++) void loader.load(`thumb${i}`).catch(() => undefined);
    const full = loader.load("full", { front: true });
    await tick();
    expect(calls.map((c) => c.url)).toEqual(["thumb0", "thumb1", "thumb2", "thumb3"]);
    calls[0].resolve();
    await tick();
    // The freed slot goes to the opened image, not to thumb4.
    expect(calls[4].url).toBe("full");
    calls[4].resolve();
    expect(await (await full).text()).toBe("full");
  });

  it("moves an already-queued request to the front when it is opened", async () => {
    const { calls, fetchBlob } = manual();
    const loader = createBlobLoader(fetchBlob, 1);
    for (let i = 0; i < 4; i++) void loader.load(`thumb${i}`).catch(() => undefined);
    void loader.load("thumb3", { front: true });
    await tick();
    calls[0].resolve();
    await tick();
    expect(calls[1].url).toBe("thumb3");
  });

  it("drops the fetches that haven't started when the page closes", async () => {
    const { calls, fetchBlob } = manual();
    const loader = createBlobLoader(fetchBlob, 2);
    const pending = Array.from({ length: 5 }, (_, i) => loader.load(`a${i}`));
    await tick();
    loader.clear();
    await expect(pending[2]).rejects.toBeInstanceOf(CancelledError);
    await expect(pending[4]).rejects.toBeInstanceOf(CancelledError);
    // The next model's images go first, right after the ones already running.
    const next = loader.load("b0");
    calls[0].resolve();
    calls[1].resolve();
    await tick();
    expect(calls.map((c) => c.url)).toEqual(["a0", "a1", "b0"]);
    calls[2].resolve();
    await next;
    expect(loader.stats()).toEqual({ running: 0, waiting: 0, cached: 1 });
  });
});
