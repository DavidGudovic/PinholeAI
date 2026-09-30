import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GenerateResult, InstalledModel, ResultImage } from "../types";

// The IPC layer, faked: each generate call waits until the test resolves it.
let pending: ((r: GenerateResult) => void) | null = null;
let pendingFail: ((e: unknown) => void) | null = null;
let pendingUpscale: ((r: ResultImage) => void) | null = null;
const discarded: string[] = [];
let failGetImage = new Set<string>();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return {
    ...real,
    generate: vi.fn(
      () =>
        new Promise<GenerateResult>((res, rej) => {
          pending = res;
          pendingFail = rej;
        }),
    ),
    upscaleImage: vi.fn(() => new Promise<ResultImage>((res) => (pendingUpscale = res))),
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

const apiMod = await import("../api");
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

  it("locks the history from the moment Edit is pressed", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "patchEdit", patch: { restylePrompt: "watercolor" } });
    const run = actions.runEdit({ mode: "restyle", model, mask: null, size: [64, 64] });
    expect(store.getState().job?.kind).toBe("edit");
    // Loading another image while it runs is refused, before and after reading the file.
    await expect(actions.importToEdit(new Blob([new Uint8Array(4)]))).rejects.toMatchObject({ code: "invalid" });
    await tick();
    pending!({ images: [img("r")] } as GenerateResult);
    await run;
    expect(store.getState().job).toBeNull();
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["a", "r"]);
  });

  it("doesn't send an edit cancelled before it reached the engine", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "patchEdit", patch: { restylePrompt: "watercolor" } });
    const run = actions.runEdit({ mode: "restyle", model, mask: new Blob([new Uint8Array(4)]), size: [64, 64] });
    await actions.cancel();
    await run;
    const api = await import("../api");
    expect(api.generate).not.toHaveBeenCalled();
    expect(store.getState().job).toBeNull();
  });

  it("drops an upscale that finishes after Reset", async () => {
    const { store, actions } = setup();
    const run = actions.upscale("x", 2);
    await tick();
    await actions.clearSession();
    pendingUpscale!(img("u"));
    await run;
    expect(store.getState().results).toEqual([]);
    expect(discarded).toContain("u");
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
    store.dispatch({ type: "patchCreate", patch: { fineTune: { negativePrompt: "blurry" } } });
    await applyPastedText("Steps: 30, Sampler: Euler a, CFG scale: 7, Seed: 5, Size: 832x1216", store, actions);
    expect(store.getState().create.prompt).toBe("a lighthouse");
    expect(store.getState().create.fineTune.negativePrompt).toBe("blurry");
    await applyPastedText("a castle in fog\nSteps: 30, Sampler: Euler a, CFG scale: 7", store, actions);
    expect(store.getState().create.prompt).toBe("a castle in fog");
  });
});

describe("job details for the screen", () => {
  it("drops an upscale cancelled while the upscaler was still downloading", async () => {
    const { store, actions } = setup();
    const run = actions.upscale("x", 2);
    await tick();
    // The engine has no job yet (the upscaler is downloading), so only the flag stops it.
    await actions.cancel();
    pendingUpscale!(img("u"));
    await run;
    expect(store.getState().results).toEqual([]);
    expect(discarded).toContain("u");
    expect(store.getState().job).toBeNull();
  });

  it("counts the placeholders from the running batch, not the How many dial", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "patchCreate", patch: { count: 4 } });
    const first = actions.generateCreate();
    await tick();
    expect(store.getState().job?.count).toBe(4);
    pending!({ images: [img("a"), img("b"), img("c"), img("d")] } as GenerateResult);
    await first;
    store.dispatch({ type: "patchCreate", patch: { count: 1 } });
    const again = actions.variations("a");
    await tick();
    expect(store.getState().job?.count).toBe(4);
    pending!({ images: [img("e")] } as GenerateResult);
    await again;
    const up = actions.upscale("a", 2);
    await tick();
    expect(store.getState().job).toMatchObject({ kind: "upscale", count: 1 });
    pendingUpscale!(img("u"));
    await up;
  });
});

describe("Use a style add-on", () => {
  const lora = (id: string, familyId: string | null) => ({
    id,
    friendlyName: id,
    familyId,
    baseModel: familyId ? "SDXL 1.0" : null,
    trainedWords: [],
    sizeBytes: 1,
    civitaiModelId: null,
    civitaiVersionId: null,
  });

  it("adds it to Create once, at the default strength, and opens Create", () => {
    const { store, actions } = setup();
    store.dispatch({ type: "setTab", tab: "models" });
    store.dispatch({ type: "setLoras", loras: [lora("film", null)] });
    actions.addLora("film");
    actions.addLora("film");
    expect(store.getState().create.loras).toEqual([{ loraId: "film", weight: 0.8 }]);
    expect(store.getState().tab).toBe("create");
  });

  it("ignores an add-on that isn't installed", () => {
    const { store, actions } = setup();
    actions.addLora("gone");
    expect(store.getState().create.loras).toEqual([]);
  });

  it("says when the add-on is made for other models", () => {
    const { store, actions } = setup();
    store.dispatch({ type: "setModels", models: [{ ...model, familyId: "flux1_dev" }] });
    store.dispatch({ type: "setLoras", loras: [lora("xl", "sdxl")] });
    actions.addLora("xl");
    expect(store.getState().toasts.at(-1)?.text).toMatch(/made for SDXL 1.0 models/);
  });
});

describe("autoEditModel", () => {
  it("prefers a dedicated edit model over a generator that can edit, unless it is too big", () => {
    const { store, actions } = setup();
    const klein: InstalledModel = { ...model, id: "k", familyId: "flux2_klein_4b", modes: ["txt2img", "img2img", "edit"], fit: "fits" };
    const qwen: InstalledModel = { ...model, id: "q", familyId: "qwen_image_edit_2511", modes: ["edit"], isEditModel: true, fit: "tight" };
    store.dispatch({ type: "setModels", models: [klein, qwen] });
    expect(actions.autoEditModel()?.id).toBe("q");
    store.dispatch({ type: "setModels", models: [klein, { ...qwen, fit: "tooBig" }] });
    expect(actions.autoEditModel()?.id).toBe("k");
    store.dispatch({ type: "setModels", models: [klein] });
    expect(actions.autoEditModel()?.id).toBe("k");
  });

  it("with two images, only models that combine them, and one that fits wins", () => {
    const { store, actions } = setup();
    const klein: InstalledModel = { ...model, id: "k", familyId: "flux2_klein_4b", modes: ["txt2img", "img2img", "edit"], multiRef: true, fit: "fits" };
    const qwen: InstalledModel = { ...model, id: "q", familyId: "qwen_image_edit_2511", modes: ["edit"], isEditModel: true, multiRef: true, fit: "tight" };
    const kontext: InstalledModel = { ...model, id: "x", familyId: "flux1_kontext", modes: ["edit"], isEditModel: true, fit: "fits" };
    store.dispatch({ type: "setModels", models: [klein, qwen, kontext] });
    expect(actions.autoEditModel(true)?.id).toBe("k");
    store.dispatch({ type: "setModels", models: [{ ...klein, fit: "tight" }, qwen, kontext] });
    expect(actions.autoEditModel(true)?.id).toBe("q");
    store.dispatch({ type: "setModels", models: [kontext] });
    expect(actions.autoEditModel(true)).toBeNull();
  });
});

describe("setLoraTriggerWords", () => {
  it("puts the add-on's chips back to the default pick in Create and Edit", async () => {
    const { store, actions } = setup();
    const lora = { id: "l1", friendlyName: "l1", familyId: null, baseModel: null, trainedWords: ["old"], sizeBytes: 1, civitaiModelId: null, civitaiVersionId: null };
    const spy = vi.spyOn(apiMod, "setLoraTriggerWords").mockResolvedValue({ ...lora, trainedWords: ["new"] });
    store.dispatch({ type: "setLoras", loras: [lora] });
    store.dispatch({ type: "patchCreate", patch: { loras: [{ loraId: "l1", weight: 1, words: ["old"] }] } });
    store.dispatch({ type: "patchEdit", patch: { loras: [{ loraId: "l1", weight: 0.5, words: ["old"] }] } });
    await actions.setLoraTriggerWords("l1", ["new"]);
    expect(store.getState().create.loras).toEqual([{ loraId: "l1", weight: 1 }]);
    expect(store.getState().edit.loras).toEqual([{ loraId: "l1", weight: 0.5 }]);
    spy.mockRestore();
  });
});

describe("queue", () => {
  const prompts = () => vi.mocked(apiMod.generate).mock.calls.map((c) => c[0].prompt);

  it("runs a Generate pressed during a job afterwards, with the settings from when it was pressed", async () => {
    const { store, actions } = setup();
    const first = actions.generateCreate();
    await tick();
    const second = actions.generateCreate();
    store.dispatch({ type: "patchCreate", patch: { prompt: "changed later" } });
    expect(store.getState().queue).toHaveLength(1);
    expect(store.getState().queue[0]).toMatchObject({ kind: "create", label: "a lighthouse" });
    pending!({ images: [img("a")] } as GenerateResult);
    await first;
    // The queued one started as soon as the first ended.
    expect(store.getState().job?.kind).toBe("create");
    expect(store.getState().queue).toEqual([]);
    await tick();
    pending!({ images: [img("b")] } as GenerateResult);
    await second;
    expect(prompts()).toEqual(["a lighthouse", "a lighthouse"]);
    expect(store.getState().results.map((r) => r.id)).toEqual(["b", "a"]);
    expect(store.getState().job).toBeNull();
  });

  it("drops a job removed from the queue without running it", async () => {
    const { store, actions } = setup();
    const first = actions.generateCreate();
    await tick();
    const second = actions.generateCreate();
    actions.removeQueued(store.getState().queue[0].id);
    await second;
    expect(store.getState().queue).toEqual([]);
    pending!({ images: [img("a")] } as GenerateResult);
    await first;
    await tick();
    expect(apiMod.generate).toHaveBeenCalledTimes(1);
    expect(store.getState().job).toBeNull();
  });

  it("Cancel stops only the running job; the next one starts", async () => {
    const { store, actions } = setup();
    const first = actions.generateCreate();
    await tick();
    const second = actions.generateCreate();
    await actions.cancel();
    pendingFail!({ code: "cancelled", message: "Cancelled.", details: null });
    await first;
    expect(store.getState().job?.kind).toBe("create");
    await tick();
    pending!({ images: [img("b")] } as GenerateResult);
    await second;
    expect(store.getState().results.map((r) => r.id)).toEqual(["b"]);
  });

  it("keeps going after a job fails, and the failed one reports its error", async () => {
    const { store, actions } = setup();
    const first = actions.generateCreate();
    await tick();
    const second = actions.generateCreate();
    pendingFail!({ code: "out_of_memory", message: "Not enough memory.", details: null });
    await expect(first).rejects.toMatchObject({ code: "out_of_memory" });
    await tick();
    pending!({ images: [img("b")] } as GenerateResult);
    await second;
    expect(store.getState().results.map((r) => r.id)).toEqual(["b"]);
  });

  it("Reset empties the queue", async () => {
    const { store, actions } = setup();
    const first = actions.generateCreate();
    await tick();
    const second = actions.generateCreate();
    await actions.clearSession();
    await second;
    expect(store.getState().queue).toEqual([]);
    pending!({ images: [img("a")] } as GenerateResult);
    await first;
    await tick();
    expect(apiMod.generate).toHaveBeenCalledTimes(1);
    expect(store.getState().job).toBeNull();
  });

  it("queues edits of the same image and keeps both results in the history", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "patchEdit", patch: { restylePrompt: "watercolor" } });
    const first = actions.runEdit({ mode: "restyle", model, mask: null, size: [64, 64] });
    store.dispatch({ type: "patchEdit", patch: { restylePrompt: "oil paint" } });
    const second = actions.runEdit({ mode: "restyle", model, mask: null, size: [64, 64] });
    expect(store.getState().queue[0]).toMatchObject({ kind: "edit", label: "oil paint", imageIds: ["a"] });
    // The history stays put while an edit waits, too.
    await expect(actions.importToEdit(new Blob([new Uint8Array(4)]))).rejects.toMatchObject({ code: "invalid" });
    await tick();
    pending!({ images: [img("r1")] } as GenerateResult);
    await first;
    await tick();
    pending!({ images: [img("r2")] } as GenerateResult);
    await second;
    expect(prompts()).toEqual(["watercolor", "oil paint"]);
    expect(vi.mocked(apiMod.generate).mock.calls[1][0].initImageId).toBe("a");
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["a", "r1", "r2"]);
    expect(store.getState().edit.index).toBe(2);
  });

  it("keeps a queued edit's second image until the edit has run, even if it was removed meanwhile", async () => {
    const { store, actions } = setup();
    const edit: InstalledModel = { ...model, id: "e", modes: ["edit"], isEditModel: true };
    store.dispatch({ type: "setModels", models: [model, edit] });
    const first = actions.generateCreate();
    await tick();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "editSetSecond", ref: ref("two") });
    store.dispatch({ type: "patchEdit", patch: { instruction: "put the logo on the mug" } });
    const queued = actions.runEdit({ mode: "instruction", model: edit, mask: null, size: [64, 64] });
    store.dispatch({ type: "editSetSecond", ref: null });
    pending!({ images: [img("c")] } as GenerateResult);
    await first;
    await tick();
    // Running now: image 2 is still held (not discarded in the engine's session).
    expect(store.getState().images.two).toBeDefined();
    expect(vi.mocked(apiMod.generate).mock.calls[1][0].refImageIds).toEqual(["a", "two"]);
    pending!({ images: [img("r")] } as GenerateResult);
    await queued;
    expect(store.getState().images.two).toBeUndefined();
  });

  it("ignores a Generate pressed while Reset is clearing the session", async () => {
    const { store, actions } = setup();
    const first = actions.generateCreate();
    await tick();
    const reset = actions.clearSession();
    await actions.generateCreate();
    expect(store.getState().queue).toEqual([]);
    pendingFail!({ code: "cancelled", message: "Cancelled.", details: null });
    await first;
    await reset;
    await tick();
    expect(apiMod.generate).toHaveBeenCalledTimes(1);
    expect(store.getState().job).toBeNull();
  });
});
