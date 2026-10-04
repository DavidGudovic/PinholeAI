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
    saveImagesTo: vi.fn(async () => ({ saved: [], failed: 0 })),
    listModels: vi.fn(async () => []),
    listLoras: vi.fn(async () => []),
    familyUi: vi.fn(async () => {
      throw new Error("no family ui in tests");
    }),
  };
});

let background = false;
const notifyDone = vi.fn();
vi.mock("./platform", async (orig) => {
  const real = await orig<typeof import("./platform")>();
  return { ...real, windowInBackground: () => background, notifyDone: (sound: boolean) => notifyDone(sound), primeSound: () => undefined, chooseFolder: vi.fn(async () => "/x") };
});

const apiMod = await import("../api");
const { createStore } = await import("./store");
const { makeActions } = await import("./actions");
const { CLOSE_STRENGTH } = await import("./request");
const { ALSO_MAX, editBusy, sheetIds } = await import("./model");

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
  background = false;
});
afterEach(() => vi.clearAllMocks());

function setup() {
  const store = createStore();
  const actions = makeActions(store);
  store.dispatch({ type: "setModels", models: [model] });
  store.dispatch({ type: "patchCreate", patch: { modelId: "m", prompt: "a lighthouse" } });
  return { store, actions };
}

describe("use as image 2", () => {
  it("keeps image 1, sets image 2 and opens Describe a change", () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "patchEdit", patch: { mode: "restyle" } });
    store.dispatch({ type: "editSetSecond", ref: ref("b") });
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "addResults", batch: null, images: [{ ...img("b"), origin: "generated" }], refs: [ref("b")] });
    actions.sendToEditSecond("b");
    const s = store.getState();
    expect(s.edit.chain.map((n) => n.imageId)).toEqual(["a"]);
    expect(s.edit.secondImageId).toBe("b");
    expect(s.edit.mode).toBe("instruction");
    expect(s.tab).toBe("edit");
  });

  it("becomes image 1 when Edit is empty", () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editSetSecond", ref: ref("b") });
    actions.sendToEditSecond("b");
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["b"]);
    expect(store.getState().edit.secondImageId).toBeNull();
  });

  it("does nothing when the image is already image 1", () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    actions.sendToEditSecond("a");
    expect(store.getState().edit.secondImageId).toBeNull();
  });
});

describe("same character", () => {
  const klein: InstalledModel = { ...model, id: "k", friendlyName: "FLUX.2 klein", modes: ["txt2img", "img2img", "edit"] };
  const withResult = (models: InstalledModel[], origin: "generated" | "imported" = "generated") => {
    const { store, actions } = setup();
    store.dispatch({ type: "setModels", models });
    store.dispatch({ type: "addResults", batch: null, images: [{ ...img("a"), origin }], refs: [ref("a")] });
    return { store, actions };
  };

  it("uses the image as Create's reference picture when the model takes one", () => {
    const { store, actions } = withResult([model, klein]);
    store.dispatch({ type: "patchCreate", patch: { modelId: "k" } });
    actions.sameCharacter("a");
    const s = store.getState();
    expect(s.create.refImageId).toBe("a");
    expect(s.create.modelId).toBe("k");
    expect(s.tab).toBe("create");
    expect(s.create.prompt).toBe("a lighthouse");
  });

  it("keeps the picked model even when it is Too big or still needs a part", () => {
    for (const picked of [{ ...klein, fit: "tooBig" as const }, { ...klein, missingComponents: ["Vision encoder"] }]) {
      const { store, actions } = withResult([model, picked]);
      store.dispatch({ type: "patchCreate", patch: { modelId: "k" } });
      actions.sameCharacter("a");
      expect(store.getState()).toMatchObject({ tab: "create", create: { modelId: "k", refImageId: "a" } });
    }
  });

  it("switches to an installed model that takes a reference picture", () => {
    const { store, actions } = withResult([model, klein]);
    actions.sameCharacter("a");
    expect(store.getState().create).toMatchObject({ modelId: "k", refImageId: "a" });
    expect(store.getState().toasts.at(-1)?.text).toMatch(/Switched to FLUX.2 klein/);
  });

  it("doesn't switch to a model that can't run now", () => {
    const { store, actions } = withResult([model, { ...klein, fit: "tooBig" }]);
    actions.sameCharacter("a");
    expect(store.getState().create.modelId).toBe("m");
    expect(store.getState().tab).toBe("edit");
  });

  it("uses a picture the user added as Create's reference picture too", () => {
    const { store, actions } = withResult([model, klein], "imported");
    store.dispatch({ type: "patchCreate", patch: { modelId: "k" } });
    actions.sameCharacter("a");
    expect(store.getState()).toMatchObject({ tab: "create", create: { modelId: "k", refImageId: "a" } });
  });

  it("opens Describe a change in Edit when no Create model can take one", () => {
    const { store, actions } = withResult([model]);
    store.dispatch({ type: "createSetRef", ref: ref("a") });
    store.dispatch({ type: "editSetSecond", ref: ref("a") });
    actions.sameCharacter("a");
    const s = store.getState();
    expect(s.edit.chain.map((n) => n.imageId)).toEqual(["a"]);
    expect(s.edit.mode).toBe("instruction");
    expect(s.edit.secondImageId).toBeNull();
    expect(s.tab).toBe("edit");
    expect(s.create).toMatchObject({ modelId: "m", refImageId: null });
    // The image keeps its id, so the backend's origin tracking carries through.
    expect(s.images.a).toBeTruthy();
  });
});

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

  it("Try again redoes the shown edit from the step before, with a new seed", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "patchEdit", patch: { restylePrompt: "watercolor", seed: 7 } });
    const first = actions.runEdit({ mode: "restyle", model, mask: null, size: [64, 64] });
    await tick();
    pending!({ images: [img("r1")] } as GenerateResult);
    await first;
    const again = actions.runEdit({ mode: "restyle", model, mask: null, size: [64, 64], from: 0, newSeed: true });
    await tick();
    const api = await import("../api");
    const calls = vi.mocked(api.generate).mock.calls;
    expect(calls[0][0].fineTune.seed).toBe(7);
    expect(calls[1][0].fineTune.seed).toBeUndefined();
    expect(calls[1][0].initImageId).toBe("a");
    pending!({ images: [img("r2")] } as GenerateResult);
    await again;
    expect(store.getState().edit.chain.map((n) => [n.imageId, n.label])).toEqual([
      ["a", "Original"],
      ["r2", "Edit 1"],
    ]);
    expect(store.getState().edit.index).toBe(1);
    // The fixed seed stays for the next normal edit.
    expect(store.getState().edit.seed).toBe(7);
  });

  it("adds an Edit-tab upscale as the next step of the edit history", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    const run = actions.upscaleEdit(2);
    expect(store.getState().job?.kind).toBe("editUpscale");
    // Loading another image while it runs is refused.
    await expect(actions.importToEdit(new Blob([new Uint8Array(4)]))).rejects.toMatchObject({ code: "invalid" });
    await tick();
    pendingUpscale!(img("u"));
    await run;
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["a", "u"]);
    expect(store.getState().edit.index).toBe(1);
    // Create's results don't get it.
    expect(store.getState().results).toEqual([]);
  });

  it("drops an Edit-tab upscale when the history changed meanwhile", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    const run = actions.upscaleEdit(4);
    await tick();
    store.dispatch({ type: "editLoad", ref: ref("b") });
    pendingUpscale!(img("u"));
    await run;
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["b"]);
    expect(discarded).toContain("u");
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

  it("prefers Qwen-Image 2.1 over the older edit models when it fits", () => {
    const { store, actions } = setup();
    const q21: InstalledModel = { ...model, id: "21", familyId: "qwen_image_21", modes: ["txt2img", "img2img", "edit"], multiRef: true, fit: "fits" };
    const qwen: InstalledModel = { ...model, id: "q", familyId: "qwen_image_edit_2511", modes: ["edit"], isEditModel: true, fit: "fits" };
    const kontext: InstalledModel = { ...model, id: "x", familyId: "flux1_kontext", modes: ["edit"], isEditModel: true, fit: "fits" };
    store.dispatch({ type: "setModels", models: [kontext, qwen, q21] });
    expect(actions.autoEditModel()?.id).toBe("21");
    expect(actions.autoEditModel(true)?.id).toBe("21");
    store.dispatch({ type: "setModels", models: [kontext, qwen, { ...q21, fit: "tight" }] });
    expect(actions.autoEditModel()?.id).toBe("q");
    store.dispatch({ type: "setModels", models: [kontext, { ...q21, fit: "tooBig" }] });
    expect(actions.autoEditModel()?.id).toBe("x");
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

  it("keeps a queued edit's second image until the edit has run and on the step it made, even if it was removed meanwhile", async () => {
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
    // The new step remembers it (Side by side); deleting that step lets it go.
    expect(store.getState().edit.chain[1]?.secondImageId).toBe("two");
    expect(store.getState().images.two).toBeDefined();
    store.dispatch({ type: "editDelete", index: 1 });
    expect(store.getState().images.two).toBeUndefined();
  });

  it("keeps Create's reference picture for Variations when the slot was cleared during the job", async () => {
    const { store, actions } = setup();
    const klein: InstalledModel = { ...model, modes: ["txt2img", "img2img", "edit"] };
    store.dispatch({ type: "setModels", models: [klein] });
    store.dispatch({ type: "createSetRef", ref: ref("pic") });
    const run = actions.generateCreate();
    await tick();
    store.dispatch({ type: "createSetRef", ref: null });
    expect(store.getState().images.pic).toBeDefined(); // the running job holds it
    pending!({ images: [img("a")] } as GenerateResult);
    await run;
    await tick();
    expect(store.getState().images.pic).toBeDefined(); // now the batch does
    expect(discarded).not.toContain("pic");
    const again = actions.variations("a");
    await tick();
    expect(vi.mocked(apiMod.generate).mock.calls[1][0].refImageIds).toEqual(["pic"]);
    pending!({ images: [img("b")] } as GenerateResult);
    await again;
  });

  it("drops a reference picture that finishes loading after Reset", async () => {
    const { store, actions } = setup();
    vi.mocked(apiMod.importImage).mockResolvedValueOnce({ id: "late", width: 8, height: 8 } as never);
    const load = actions.importCreateReference(new Blob([new Uint8Array(4)]));
    await actions.clearSession();
    await load;
    await tick();
    expect(store.getState().create.refImageId).toBeNull();
    expect(discarded).toContain("late");
  });

  it("drops Edit and Describe images that finish loading after Reset", async () => {
    const { store, actions } = setup();
    const late = (id: string) => vi.mocked(apiMod.importImage).mockResolvedValueOnce({ id, width: 8, height: 8 } as never);
    late("e1");
    const edit = actions.importToEdit(new Blob([new Uint8Array(4)]));
    await actions.clearSession();
    await edit;
    late("e2");
    const second = actions.importSecondToEdit(new Blob([new Uint8Array(4)]));
    await actions.clearSession();
    await second;
    late("d");
    const describe = actions.importToDescribe(new Blob([new Uint8Array(4)]));
    await actions.clearSession();
    await describe;
    await tick();
    expect(store.getState().edit.chain).toEqual([]);
    expect(store.getState().edit.secondImageId).toBeNull();
    expect(store.getState().describe.imageId).toBeNull();
    expect(discarded).toEqual(expect.arrayContaining(["e1", "e2", "d"]));
  });

  it("holds an upscale's source so its result keeps the settings for Variations when the source is removed meanwhile", async () => {
    const { store, actions } = setup();
    const gen = actions.generateCreate();
    await tick();
    pending!({ images: [img("a")] } as GenerateResult);
    await gen;
    const run = actions.upscale("a", 2);
    actions.removeResult("a");
    await tick();
    pendingUpscale!(img("u"));
    await run;
    expect(store.getState().results.map((r) => r.id)).toEqual(["u"]);
    expect(store.getState().resultBatch.u).toBeDefined();
    const again = actions.variations("u");
    await tick();
    expect(vi.mocked(apiMod.generate).mock.calls[1][0].prompt).toBe("a lighthouse");
    pending!({ images: [img("v")] } as GenerateResult);
    await again;
  });

  it("queues an Upscale pressed during a job and runs it afterwards, keeping its source while it waits", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "addResults", batch: null, images: [img("a")], refs: [ref("a")] });
    const second = actions.generateCreate();
    await tick();
    const up = actions.upscale("a", 2);
    expect(store.getState().queue).toEqual([expect.objectContaining({ kind: "upscale", label: "Upscale 2×", detail: "128×128", imageIds: ["a"] })]);
    // Removed while it waits: the queue still holds the picture it reads.
    actions.removeResult("a");
    expect(store.getState().images.a).toBeDefined();
    expect(apiMod.upscaleImage).not.toHaveBeenCalled();
    pending!({ images: [img("b")] } as GenerateResult);
    await second;
    expect(store.getState().job?.kind).toBe("upscale");
    expect(apiMod.upscaleImage).toHaveBeenCalledWith("a", 2);
    await tick();
    pendingUpscale!(img("u"));
    await up;
    expect(store.getState().results.map((r) => r.id)).toEqual(["u", "b"]);
    expect(store.getState().job).toBeNull();
  });

  it("queues an Edit-tab Upscale behind an edit and adds it after the edit's result", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "patchEdit", patch: { restylePrompt: "watercolor" } });
    const edit = actions.runEdit({ mode: "restyle", model, mask: null, size: [64, 64] });
    const up = actions.upscaleEdit(4);
    expect(store.getState().queue).toEqual([expect.objectContaining({ kind: "editUpscale", label: "Upscale 4×", imageIds: ["a"] })]);
    await tick();
    pending!({ images: [img("r1")] } as GenerateResult);
    await edit;
    // Still waiting or running: the history stays put.
    await expect(actions.importToEdit(new Blob([new Uint8Array(4)]))).rejects.toMatchObject({ code: "invalid" });
    await tick();
    pendingUpscale!(img("u"));
    await up;
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["a", "r1", "u"]);
    expect(store.getState().edit.index).toBe(2);
  });

  it("drops a queued upscale removed from the queue without running it", async () => {
    const { store, actions } = setup();
    const gen = actions.generateCreate();
    await tick();
    store.dispatch({ type: "addResults", batch: null, images: [img("x")], refs: [ref("x")] });
    const up = actions.upscale("x", 2);
    actions.removeQueued(store.getState().queue[0].id);
    await up;
    pending!({ images: [img("a")] } as GenerateResult);
    await gen;
    await tick();
    expect(apiMod.upscaleImage).not.toHaveBeenCalled();
    expect(store.getState().job).toBeNull();
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

describe("unsaved pictures", () => {
  it("lets Reset and close go ahead when everything is saved, and asks otherwise", () => {
    const { store, actions } = setup();
    expect(actions.requestLeave("close")).toBe(true);
    store.dispatch({ type: "addResults", batch: null, images: [img("a")], refs: [ref("a")] });
    expect(actions.requestLeave("close")).toBe(false);
    expect(store.getState().leave).toBe("close");
    store.dispatch({ type: "askLeave", what: null });
    store.dispatch({ type: "markSaved", entries: [{ id: "a", path: "/x/a.png" }] });
    expect(actions.requestLeave("clear")).toBe(true);
    expect(store.getState().leave).toBeNull();
  });

  it("asks before another image replaces unsaved edits, and loads it when the user goes ahead", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "editPush", ref: ref("r"), meta: img("r") });
    store.dispatch({ type: "addResults", batch: null, images: [img("c")], refs: [ref("c")] });
    vi.mocked(apiMod.importImage).mockResolvedValueOnce({ id: "pasted", width: 8, height: 8 } as never);
    expect(await actions.importToEdit(new Blob([new Uint8Array(4)]))).toBe(false);
    expect(store.getState().leave).toBe("edit");
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["a", "r"]);
    expect(apiMod.importImage).not.toHaveBeenCalled();
    // "Go back" keeps the edits.
    store.dispatch({ type: "askLeave", what: null });
    // Other ways into Edit ask too.
    actions.sendToEdit("c");
    expect(store.getState().leave).toBe("edit");
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["a", "r"]);
    await actions.finishLeave("edit");
    expect(store.getState().leave).toBeNull();
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["c"]);
    expect(store.getState().images.r).toBeUndefined();
  });

  it("replaces the Edit history without asking when its edits are saved", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "editPush", ref: ref("r"), meta: img("r") });
    store.dispatch({ type: "markSaved", entries: [{ id: "r", path: "/x/r.png" }] });
    vi.mocked(apiMod.importImage).mockResolvedValueOnce({ id: "pasted", width: 8, height: 8 } as never);
    expect(await actions.importToEdit(new Blob([new Uint8Array(4)]))).toBe(true);
    expect(store.getState().leave).toBeNull();
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["pasted"]);
  });

  it("Save all can save just the given pictures", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "addResults", batch: null, images: [img("a"), img("b")], refs: [ref("a"), ref("b")] });
    vi.mocked(apiMod.saveImagesTo).mockResolvedValueOnce({ saved: [{ id: "b", path: "/x/b.png" }], failed: 0 } as never);
    expect(await actions.saveAll(["b"])).toBe(true);
    expect(apiMod.saveImagesTo).toHaveBeenCalledWith(["b"], "/x");
  });

  it("remembers the prompt of each Generate for Up/Down recall", async () => {
    const { store, actions } = setup();
    const run = actions.generateCreate();
    await tick();
    pending!({ images: [img("a")] } as GenerateResult);
    await run;
    expect(store.getState().promptHistory).toEqual(["a lighthouse"]);
  });
});

describe("done alert", () => {
  it("flashes when a picture finishes while the window is in the background", async () => {
    const { store, actions } = setup();
    background = true;
    store.dispatch({ type: "setSettings", settings: { soundOnDone: true } as never });
    const run = actions.generateCreate();
    await tick();
    pending!({ images: [img("a")] } as GenerateResult);
    await run;
    expect(notifyDone).toHaveBeenCalledWith(true);
  });

  it("stays quiet while the window is in front, on cancel, and until the queue is empty", async () => {
    const { store, actions } = setup();
    // The fake listModels() returns nothing, which empties the list after each batch.
    const restore = async () => {
      await tick();
      store.dispatch({ type: "setModels", models: [model] });
    };
    const first = actions.generateCreate();
    await tick();
    pending!({ images: [img("a")] } as GenerateResult);
    await first;
    await restore();
    expect(notifyDone).not.toHaveBeenCalled();

    background = true;
    const cancelled = actions.generateCreate();
    await tick();
    pendingFail!({ code: "cancelled", message: "Cancelled.", details: null });
    await cancelled;
    await restore();
    expect(notifyDone).not.toHaveBeenCalled();

    const one = actions.generateCreate();
    await tick();
    const two = actions.generateCreate(); // queued behind the first
    pending!({ images: [img("b")] } as GenerateResult);
    await one;
    expect(notifyDone).not.toHaveBeenCalled();
    await tick();
    pending!({ images: [img("c")] } as GenerateResult);
    await two;
    expect(notifyDone).toHaveBeenCalledTimes(1);
  });
});

describe("Close to this one and Finish at Best quality", () => {
  it("Close to this one restyles the picture with the batch's settings, How many times, and keeps its source", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "patchCreate", patch: { quality: "fast", fineTune: { seed: 9 } } });
    const first = actions.generateCreate();
    await tick();
    pending!({ images: [{ ...img("a"), width: 96, height: 64 }] } as GenerateResult);
    await first;
    store.dispatch({ type: "patchCreate", patch: { count: 2 } });
    const close = actions.closeTo("a");
    await tick();
    const req = vi.mocked(apiMod.generate).mock.calls[1][0];
    expect(req).toMatchObject({ mode: "img2img", prompt: "a lighthouse", modelId: "m", initImageId: "a", strength: CLOSE_STRENGTH, refImageIds: [] });
    expect(req.dials).toMatchObject({ quality: "fast", count: 2 });
    expect(req.fineTune.seed).toBeUndefined();
    expect([req.fineTune.width, req.fineTune.height]).toEqual([96, 64]);
    expect(store.getState().queue).toEqual([]);
    pending!({ images: [img("c1"), img("c2")] } as GenerateResult);
    await close;
    // The new batch holds its source, so Variations of it still work after it is removed.
    actions.removeResult("a");
    expect(store.getState().images.a).toBeDefined();
    const again = actions.variations("c1");
    await tick();
    expect(vi.mocked(apiMod.generate).mock.calls[2][0]).toMatchObject({ mode: "img2img", initImageId: "a" });
    pending!({ images: [img("c3")] } as GenerateResult);
    await again;
    // A queued upscale of a Close picture holds the source too.
    const job = actions.variations("c1");
    await tick();
    const up = actions.upscale("c1", 2);
    expect(store.getState().queue[0].imageIds).toEqual(["c1", "a"]);
    pending!({ images: [img("c4")] } as GenerateResult);
    await job;
    await tick();
    pendingUpscale!(img("u"));
    await up;
  });

  it("Finish at Best quality re-makes one picture at Best with its own seed", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "patchCreate", patch: { quality: "balanced", count: 2, fineTune: { steps: 8 } } });
    const first = actions.generateCreate();
    await tick();
    pending!({ images: [{ ...img("a"), seed: 40 }, { ...img("b"), seed: 41 }] } as GenerateResult);
    await first;
    const finish = actions.finishAtBest("b");
    await tick();
    const req = vi.mocked(apiMod.generate).mock.calls[1][0];
    expect(req).toMatchObject({ mode: "txt2img", prompt: "a lighthouse", modelId: "m" });
    expect(req.dials).toMatchObject({ quality: "best", count: 1 });
    expect(req.fineTune.seed).toBe(41);
    expect(req.fineTune.steps).toBeUndefined();
    pending!({ images: [img("f")] } as GenerateResult);
    await finish;
  });

  it("Finish at Best quality does nothing for a picture already made at Best", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "patchCreate", patch: { quality: "best" } });
    const first = actions.generateCreate();
    await tick();
    pending!({ images: [img("a")] } as GenerateResult);
    await first;
    await actions.finishAtBest("a");
    expect(vi.mocked(apiMod.generate)).toHaveBeenCalledTimes(1);
  });
});

describe("choices in braces", () => {
  const calls = () => vi.mocked(apiMod.generate).mock.calls.map((c) => c[0]);

  it("makes one picture per choice as one job, with one seed, keeping the typed prompt in history", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "patchCreate", patch: { prompt: "a {red|blue|green} car", count: 4 } });
    const run = actions.generateCreate();
    await tick();
    expect(store.getState().job?.count).toBe(3);
    for (const [i, id] of ["a", "b", "c"].entries()) {
      await tick();
      pending!({ images: [img(id)] } as GenerateResult);
      await tick();
      await tick();
      expect(store.getState().job?.count ?? 0).toBe(2 - i);
    }
    await run;
    expect(calls().map((r) => r.prompt)).toEqual(["a red car", "a blue car", "a green car"]);
    expect(calls().every((r) => r.dials.count === 1)).toBe(true);
    const seeds = new Set(calls().map((r) => r.fineTune.seed));
    expect(seeds.size).toBe(1);
    expect(typeof [...seeds][0]).toBe("number");
    expect(store.getState().results.map((r) => r.id)).toEqual(["c", "b", "a"]);
    // Each picture's Variations use its own prompt.
    const s = store.getState();
    expect(s.batches[s.resultBatch["b"]].request.prompt).toBe("a blue car");
    expect(s.promptHistory).toEqual(["a {red|blue|green} car"]);
    expect(s.job).toBeNull();
  });

  it("keeps making the other choices when one fails, then shows the error", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "patchCreate", patch: { prompt: "{a|b|c}" } });
    const run = actions.generateCreate();
    await tick();
    pending!({ images: [img("a")] } as GenerateResult);
    await tick();
    await tick();
    pendingFail!({ code: "invalid", message: "No.", details: null });
    await tick();
    await tick();
    pending!({ images: [img("c")] } as GenerateResult);
    await expect(run).rejects.toMatchObject({ code: "invalid" });
    expect(calls().map((r) => r.prompt)).toEqual(["a", "b", "c"]);
    expect(store.getState().results.map((r) => r.id)).toEqual(["c", "a"]);
  });

  it("refuses choices that leave nothing to make", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "patchCreate", patch: { prompt: "{|}" } });
    await expect(actions.generateCreate()).rejects.toMatchObject({ code: "invalid" });
    expect(apiMod.generate).not.toHaveBeenCalled();
  });

  it("uses the locked seed, and Cancel keeps the pictures already made", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "patchCreate", patch: { prompt: "{a|b|c}", fineTune: { seed: 7 } } });
    const run = actions.generateCreate();
    await tick();
    pending!({ images: [img("a")] } as GenerateResult);
    await tick();
    await tick();
    await actions.cancel();
    pendingFail!({ code: "cancelled", message: "Cancelled.", details: null });
    await run;
    expect(calls().map((r) => r.fineTune.seed)).toEqual([7, 7]);
    expect(store.getState().results.map((r) => r.id)).toEqual(["a"]);
    expect(store.getState().job).toBeNull();
  });
});

describe("on another model", () => {
  const other: InstalledModel = { ...model, id: "o", friendlyName: "Other" };

  it("runs a Close to this one picture's prompt on its own there", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "setModels", models: [model, other] });
    const first = actions.generateCreate();
    await tick();
    pending!({ images: [img("a")] } as GenerateResult);
    await first;
    const close = actions.closeTo("a");
    await tick();
    pending!({ images: [{ ...img("c"), seed: 5 }] } as GenerateResult);
    await close;
    store.dispatch({ type: "setModels", models: [model, other] });
    const run = actions.onOtherModel("c", "o");
    await tick();
    pending!({ images: [img("d")] } as GenerateResult);
    await run;
    const req = vi.mocked(apiMod.generate).mock.calls[2][0];
    expect(req).toMatchObject({ modelId: "o", mode: "txt2img", initImageId: null, strength: null });
    expect(req.fineTune.seed).toBe(5);
  });

  it("runs the same prompt, seed and shape on the other model, without the first model's own settings", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "setModels", models: [model, other] });
    store.dispatch({ type: "patchCreate", patch: { shape: "portrait", count: 2, fineTune: { sampler: "euler", steps: 30, width: 512, height: 1024, negativePrompt: "blur" } } });
    const first = actions.generateCreate();
    await tick();
    pending!({ images: [{ ...img("a"), seed: 42 }, img("b")] } as GenerateResult);
    await first;
    await tick();
    // The refresh after a job reads the (empty) fake model list.
    store.dispatch({ type: "setModels", models: [model, other] });

    const run = actions.onOtherModel("a", "o");
    await tick();
    expect(store.getState().job?.count).toBe(1);
    pending!({ images: [img("c")] } as GenerateResult);
    await run;
    const req = vi.mocked(apiMod.generate).mock.calls[1][0];
    expect(req).toMatchObject({ modelId: "o", prompt: "a lighthouse", dials: { shape: "portrait", count: 1 } });
    const { width, height, ...rest } = req.fineTune;
    expect(rest).toEqual({ seed: 42, negativePrompt: "blur" });
    // The set size keeps its shape, at the new model's usual size.
    expect(width! / height!).toBeCloseTo(0.5, 1);
    expect(width! * height!).toBeGreaterThan(512 * 1024 * 0.9);
    const s = store.getState();
    expect(s.selectedResultId).toBe("c");
    expect(s.batches[s.resultBatch["c"]].compareWith).toBe("a");
  });

  it("refuses a model that can't use the picture's reference picture", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "addResults", batch: { id: "b", request: { modelId: "m", prompt: "p", refImageIds: ["r"] } as never }, images: [img("a")], refs: [ref("a")] });
    store.dispatch({ type: "setModels", models: [model, other] });
    await expect(actions.onOtherModel("a", "o")).rejects.toMatchObject({ code: "invalid" });
    expect(apiMod.generate).not.toHaveBeenCalled();
  });

  it("says when the picture's settings are gone", async () => {
    const { actions } = setup();
    await expect(actions.onOtherModel("x", "o")).rejects.toMatchObject({ code: "not_found" });
  });
});

describe("Also apply to…", () => {
  it("queues the same edit on each picture after the shown one; the run's results go to Create together", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "editSetAlso", refs: [ref("b"), ref("c")] });
    store.dispatch({ type: "patchEdit", patch: { restylePrompt: "watercolor" } });
    const sizes: [number, number][] = [];
    const run = actions.runEdit({
      mode: "restyle",
      model,
      mask: null,
      size: [64, 64],
      alsoSize: (w, h) => (sizes.push([w, h]), [w, h]),
    });
    // The list is handed to the queue: the pictures stay held by it.
    expect(store.getState().edit.alsoIds).toEqual([]);
    expect(store.getState().queue.map((q) => q.imageIds)).toEqual([["b"], ["c"]]);
    expect(store.getState().images.b).toBeDefined();
    await tick();
    pending!({ images: [img("ra")] } as GenerateResult);
    await run;
    for (const out of ["rb", "rc"]) {
      await tick();
      await tick();
      pending!({ images: [img(out)] } as GenerateResult);
    }
    await tick();
    await tick();
    const calls = vi.mocked(apiMod.generate).mock.calls.map((c) => c[0]);
    expect(calls.map((r) => r.initImageId)).toEqual(["a", "b", "c"]);
    expect(calls.map((r) => r.maskImageId ?? null)).toEqual([null, null, null]);
    expect(sizes).toEqual([[64, 64], [64, 64]]);
    const s = store.getState();
    expect(s.edit.chain.map((n) => n.imageId)).toEqual(["a", "ra"]);
    expect(s.results.map((r) => r.id)).toEqual(["rc", "rb", "ra"]);
    expect(new Set(["ra", "rb", "rc"].map((id) => s.resultGroup[id])).size).toBe(1);
    expect(sheetIds(s, "rb")).toEqual(["ra", "rb", "rc"]);
  });

  it("Describe a change leaves image 2 out of the other pictures", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "editSetAlso", refs: [ref("b"), ref("c")] });
    store.dispatch({ type: "patchEdit", patch: { secondImageId: "b", instruction: "make it night" } });
    void actions.runEdit({ mode: "instruction", model, mask: null, size: [64, 64], alsoSize: (w, h) => [w, h] }).catch(() => undefined);
    expect(store.getState().queue.map((q) => q.imageIds[0])).toEqual(["c"]);
  });

  it("Cancel stops the whole run; the other pictures don't lock the history", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "editSetAlso", refs: [ref("b"), ref("c")] });
    store.dispatch({ type: "patchEdit", patch: { restylePrompt: "watercolor" } });
    const run = actions.runEdit({ mode: "restyle", model, mask: null, size: [64, 64], alsoSize: (w, h) => [w, h] });
    expect(store.getState().queue.map((q) => q.kind)).toEqual(["editMore", "editMore"]);
    await tick();
    pending!({ images: [img("ra")] } as GenerateResult);
    await run;
    await tick();
    // Picture b runs now; the history is free again.
    expect(store.getState().job?.kind).toBe("editMore");
    expect(editBusy(store.getState())).toBe(false);
    await actions.cancel();
    expect(store.getState().queue).toEqual([]);
    pendingFail!({ code: "cancelled", message: "Cancelled.", details: null });
    await tick();
    await tick();
    expect(vi.mocked(apiMod.generate)).toHaveBeenCalledTimes(2);
    expect(store.getState().job).toBeNull();
  });

  it("Try again redoes only the shown picture", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "patchEdit", patch: { restylePrompt: "watercolor" } });
    const first = actions.runEdit({ mode: "restyle", model, mask: null, size: [64, 64] });
    await tick();
    pending!({ images: [img("r")] } as GenerateResult);
    await first;
    store.dispatch({ type: "editSetAlso", refs: [ref("b")] });
    const again = actions.runEdit({ mode: "restyle", model, mask: null, size: [64, 64], from: 0, newSeed: true, alsoSize: (w, h) => [w, h] });
    expect(store.getState().queue).toEqual([]);
    await tick();
    pending!({ images: [img("r2")] } as GenerateResult);
    await again;
    expect(vi.mocked(apiMod.generate)).toHaveBeenCalledTimes(2);
    expect(store.getState().edit.alsoIds).toEqual(["b"]);
  });

  it("adds picture files up to the limit", async () => {
    const { store, actions } = setup();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    let n = 0;
    vi.mocked(apiMod.importImage).mockImplementation(async () => ({ id: `f${n++}`, width: 8, height: 8 }) as never);
    const files = Array.from({ length: ALSO_MAX + 2 }, () => new Blob([new Uint8Array(4)]));
    await actions.importAlsoToEdit(files);
    expect(store.getState().edit.alsoIds).toHaveLength(ALSO_MAX);
    await actions.importAlsoToEdit([new Blob([new Uint8Array(4)])]);
    expect(store.getState().edit.alsoIds).toHaveLength(ALSO_MAX);
  });
});

describe("Save as one sheet", () => {
  it("asks where and saves the pictures as one", async () => {
    const { store, actions } = setup();
    const saveSheetAs = vi.spyOn(apiMod, "saveSheetAs").mockResolvedValue({ path: "/x/sheet.png" });
    vi.spyOn(apiMod, "saveFolderInfo").mockResolvedValue({ path: "/home/u/Pictures/Pinhole", custom: false, defaultPath: "/home/u/Pictures/Pinhole", earlierPath: null });
    const platform = await import("./platform");
    vi.spyOn(platform, "canSaveAs").mockReturnValue(true);
    vi.spyOn(platform, "chooseSavePath").mockResolvedValue("/x/sheet.png");
    await actions.saveSheet(["a", "b"]);
    expect(saveSheetAs).toHaveBeenCalledWith(["a", "b"], "/x/sheet.png");
    // The dialog starts in the Saved pictures folder.
    expect(platform.chooseSavePath).toHaveBeenCalledWith("pinhole_sheet_2.png", "/home/u/Pictures/Pinhole");
    // The pictures themselves still count as unsaved.
    expect(store.getState().saved).toEqual({});
  });
});

describe("refreshModels", () => {
  const deferred = <T,>() => {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
      resolve = res;
      reject = rej;
    });
    return { promise, resolve, reject };
  };
  const other: InstalledModel = { ...model, id: "n", friendlyName: "n" };

  it("keeps the newest list when an older reply arrives last", async () => {
    const { store, actions } = setup();
    const older = deferred<InstalledModel[]>();
    const newer = deferred<InstalledModel[]>();
    vi.mocked(apiMod.listModels).mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise);
    const first = actions.refreshModels();
    const second = actions.refreshModels();
    newer.resolve([model]);
    await second;
    older.resolve([model, other]);
    await first;
    expect(store.getState().models?.map((m) => m.id)).toEqual(["m"]);
  });

  it("ignores an error from an older refresh", async () => {
    const { store, actions } = setup();
    const older = deferred<InstalledModel[]>();
    vi.mocked(apiMod.listModels).mockReturnValueOnce(older.promise).mockResolvedValueOnce([model, other]);
    const first = actions.refreshModels();
    await actions.refreshModels();
    older.reject({ code: "io", message: "failed", details: null });
    await expect(first).resolves.toBeUndefined();
    expect(store.getState().models?.map((m) => m.id)).toEqual(["m", "n"]);
  });
});
