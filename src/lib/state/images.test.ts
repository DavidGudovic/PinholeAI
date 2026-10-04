import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ResultImage } from "../types";

vi.mock("../api", async (orig) => ({ ...(await orig<typeof import("../api")>()), discardImage: vi.fn(async () => undefined) }));

const { THUMBS_AT_ONCE, imageMime, makeThumbUrl, releaseRefs, watchThumbs } = await import("./images");
const { createStore } = await import("./store");

const buf = (...bytes: number[]) => new Uint8Array([...bytes, ...new Array(16).fill(0)]).buffer;

describe("imageMime", () => {
  it("types session bytes by their magic number (imported images keep their format)", () => {
    expect(imageMime(buf(0x89, 0x50, 0x4e, 0x47))).toBe("image/png");
    expect(imageMime(buf(0xff, 0xd8, 0xff, 0xe0))).toBe("image/jpeg");
    expect(imageMime(buf(0x52, 0x49, 0x46, 0x46, 1, 2, 3, 4, 0x57, 0x45, 0x42, 0x50))).toBe("image/webp");
    expect(imageMime(new ArrayBuffer(0))).toBe("image/png");
  });
});

describe("thumbnails", () => {
  const ref = (id: string) => ({ id, url: `blob:${id}`, width: 2048, height: 1024 });
  const made = (id: string) => ({ id, width: 2048, height: 1024, seed: 1, modelId: "m", modelLabel: "M", familyId: "sdxl", steps: 1, cfg: 1, guidance: null, sampler: null, scheduler: null, parentId: null }) as ResultImage;
  const hold = () => {
    let resolve!: (v: string | null) => void;
    const promise = new Promise<string | null>((r) => (resolve = r));
    return { promise, resolve };
  };
  let revoked: string[];
  beforeEach(() => {
    revoked = [];
    vi.useFakeTimers();
    vi.spyOn(URL, "revokeObjectURL").mockImplementation((u: string) => void revoked.push(u));
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  const setup = () => {
    const store = createStore((refs, a) => releaseRefs(refs, a.type !== "clearSession"));
    const pending = new Map<string, ReturnType<typeof hold>>();
    const make = vi.fn((r: { id: string }) => {
      const h = hold();
      pending.set(r.id, h);
      return h.promise;
    });
    watchThumbs(store, make);
    return { store, make, pending };
  };

  it("adds a small copy to a ref once it is added, and revokes it with the picture", async () => {
    const { store, make, pending } = setup();
    store.dispatch({ type: "addResults", batch: null, images: [made("a")], refs: [ref("a")] });
    // The ref is in the store before its thumbnail exists.
    expect(store.getState().images.a.thumbUrl).toBeUndefined();
    expect(make).toHaveBeenCalledTimes(1);
    store.dispatch({ type: "selectResult", id: "a" });
    pending.get("a")!.resolve("blob:a-thumb");
    await vi.waitFor(() => expect(store.getState().images.a.thumbUrl).toBe("blob:a-thumb"));
    expect(store.getState().images.a.url).toBe("blob:a");
    expect(make).toHaveBeenCalledTimes(1);
    // The same picture added again (Edit) keeps its thumbnail.
    store.dispatch({ type: "editLoad", ref: ref("a") });
    expect(store.getState().images.a.thumbUrl).toBe("blob:a-thumb");
    store.dispatch({ type: "editClear" });
    store.dispatch({ type: "removeResult", id: "a" });
    vi.runAllTimers();
    expect(revoked.sort()).toEqual(["blob:a", "blob:a-thumb"]);
  });

  it("revokes a thumbnail that is ready only after its picture was removed", async () => {
    const { store, pending } = setup();
    store.dispatch({ type: "addResults", batch: null, images: [made("a")], refs: [ref("a")] });
    store.dispatch({ type: "removeResult", id: "a" });
    vi.runAllTimers();
    expect(revoked).toEqual(["blob:a"]);
    pending.get("a")!.resolve("blob:a-thumb");
    await vi.waitFor(() => expect(revoked).toEqual(["blob:a", "blob:a-thumb"]));
    expect(store.getState().images.a).toBeUndefined();
  });

  it("makes thumbnails one at a time and skips a picture removed while it waits", async () => {
    const { store, make, pending } = setup();
    const ids = ["a", "b", "c", "d", "e", "f"];
    store.dispatch({ type: "addResults", batch: null, images: ids.map(made), refs: ids.map(ref) });
    expect(make).toHaveBeenCalledTimes(THUMBS_AT_ONCE);
    const waiting = ids.find((id) => !pending.has(id))!;
    store.dispatch({ type: "removeResult", id: waiting });
    for (let k = 1; k <= 5; k++) {
      await vi.waitFor(() => expect(make).toHaveBeenCalledTimes(k));
      const id = make.mock.calls[k - 1][0].id;
      pending.get(id)!.resolve(`blob:${id}-thumb`);
      await vi.waitFor(() => expect(store.getState().images[id].thumbUrl).toBe(`blob:${id}-thumb`));
    }
    expect(make).toHaveBeenCalledTimes(5);
    expect(make.mock.calls.map(([r]) => r.id)).not.toContain(waiting);
  });

  it("makes no copy of a picture that is already small", async () => {
    expect(await makeThumbUrl({ id: "s", url: "blob:s", width: 160, height: 90 })).toBeNull();
  });
});
