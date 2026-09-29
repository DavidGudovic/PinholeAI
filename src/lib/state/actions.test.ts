import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GenerateResult, InstalledModel, ResultImage } from "../types";

// The IPC layer, faked: each generate call waits until the test resolves it.
let pending: ((r: GenerateResult) => void) | null = null;
const discarded: string[] = [];
let failGetImage = new Set<string>();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    generate: vi.fn(() => new Promise<GenerateResult>((res) => (pending = res))),
    getImage: vi.fn(async (id: string) => {
      if (failGetImage.has(id)) throw { code: "not_found", message: "gone", details: null };
      return new ArrayBuffer(8);
    }),
    discardImage: vi.fn(async (id: string) => void discarded.push(id)),
    importImage: vi.fn(async () => ({ id: "mask", width: 8, height: 8 })),
    cancelGeneration: vi.fn(async () => undefined),
    clearSession: vi.fn(async () => undefined),
    listModels: vi.fn(async () => []),
    listLoras: vi.fn(async () => []),
    familyUi: vi.fn(async () => {
      throw new Error("no family ui in tests");
    }),
  };
});

const { createStore } = await import("./store");
const { makeActions } = await import("./actions");

const model: InstalledModel = {
  id: "m",
  friendlyName: "m",
  familyId: null,
  familyLabel: "m",
  styleBadge: null,
  modes: ["txt2img", "img2img"],
  isEditModel: false,
  sizeBytes: 1,
  vram: null,
  fit: null,
  lastUsed: null,
  missingComponents: [],
  licenseNote: null,
  civitaiModelId: null,
  civitaiVersionId: null,
  baseModel: null,
};
const img = (id: string): ResultImage => ({ id, width: 64, height: 64, seed: 1, model: "m", steps: 1, cfg: 1, sampler: "euler", scheduler: "simple" }) as unknown as ResultImage;
const ref = (id: string) => ({ id, url: `blob:${id}`, width: 64, height: 64 });
const tick = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  pending = null;
  discarded.length = 0;
  failGetImage = new Set();
  globalThis.URL.createObjectURL = vi.fn((b: Blob) => `blob:${b.size}`) as typeof URL.createObjectURL;
  globalThis.URL.revokeObjectURL = vi.fn();
});
afterEach(() => vi.clearAllMocks());

function setup() {
  const store = createStore();
  const actions = makeActions(store);
  store.dispatch({ type: "setModels", models: [model] });
  store.dispatch({ type: "patchCreate", patch: { modelId: "m", prompt: "a lighthouse" } });
  return { store, actions };
}

describe("async results after the screen moved on", () => {
  it("drops an edit result when another image was loaded while it ran", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "patchEdit", patch: { restylePrompt: "watercolor" } });
    const run = actions.runEdit({ mode: "restyle", model, mask: null, size: [64, 64] });
    await tick();
    store.dispatch({ type: "editLoad", ref: ref("b") });
    pending!({ images: [img("r")] } as GenerateResult);
    await run;
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["b"]);
    expect(discarded).toContain("r");
  });

  it("adds an edit result after the image it was made from", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "patchEdit", patch: { restylePrompt: "watercolor" } });
    const run = actions.runEdit({ mode: "restyle", model, mask: null, size: [64, 64] });
    await tick();
    pending!({ images: [img("r")] } as GenerateResult);
    await run;
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["a", "r"]);
  });

  it("drops generated images that finish after Reset", async () => {
    const { store, actions } = setup();
    const run = actions.generateCreate();
    await tick();
    await actions.clearSession();
    pending!({ images: [img("x"), img("y")] } as GenerateResult);
    await run;
    expect(store.getState().results).toEqual([]);
    expect(discarded).toEqual(expect.arrayContaining(["x", "y"]));
  });

  it("releases the other images of a batch when one can't be read", async () => {
    const { store, actions } = setup();
    failGetImage.add("y");
    const run = actions.generateCreate();
    await tick();
    pending!({ images: [img("x"), img("y")] } as GenerateResult);
    await expect(run).rejects.toMatchObject({ code: "not_found" });
    expect(store.getState().results).toEqual([]);
    expect(discarded).toEqual(expect.arrayContaining(["x", "y"]));
    expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
  });
});

describe("Paste from CivitAI", () => {
  it("keeps the typed prompt when only settings are pasted", async () => {
    const { applyPastedText } = await import("../../tabs/create/pasteApply");
    const { store, actions } = setup();
    await applyPastedText("Steps: 30, Sampler: Euler a, CFG scale: 7, Seed: 5, Size: 832x1216", store, actions);
    expect(store.getState().create.prompt).toBe("a lighthouse");
    await applyPastedText("a castle in fog\nSteps: 30, Sampler: Euler a, CFG scale: 7", store, actions);
    expect(store.getState().create.prompt).toBe("a castle in fog");
  });
});
