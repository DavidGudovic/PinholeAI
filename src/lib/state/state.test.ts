import { describe, expect, it } from "vitest";
import type { InstalledLora, InstalledModel, Preset, ResultImage } from "../types";
import { FAMILY_UI } from "./familyFixtures";
import { initialState, reducer, referencedImageIds, type Action, type AppState, type ImgRef } from "./model";
import { applyPreset, clearPreset, buildCreateRequest, buildEditRequest, editOutputSize, fitEditSize, presetFromCreate, settingsSummary, variationRequest } from "./request";

const SENTINEL = "PINHOLE_SENTINEL_7f3a";

const model = (id: string, familyId: string, extra: Partial<InstalledModel> = {}): InstalledModel => ({
  id,
  friendlyName: id,
  familyId,
  familyLabel: familyId,
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
  ...extra,
});

const lora = (id: string, familyId: string | null, civitaiVersionId: number | null = null): InstalledLora => ({
  id,
  friendlyName: id,
  familyId,
  baseModel: null,
  trainedWords: [],
  sizeBytes: 1,
  civitaiModelId: null,
  civitaiVersionId,
});

const result = (id: string, seed: number, extra: Partial<ResultImage> = {}): ResultImage => ({
  id,
  width: 1024,
  height: 1024,
  seed,
  modelId: "m1",
  modelLabel: "Model One",
  familyId: "sdxl",
  steps: 30,
  cfg: 6,
  guidance: null,
  sampler: "dpm++2m",
  scheduler: "karras",
  parentId: null,
  ...extra,
});

const ref = (id: string): ImgRef => ({ id, url: `blob:${id}`, width: 64, height: 64 });

function run(s: AppState, ...actions: Action[]): AppState {
  return actions.reduce(reducer, s);
}

function withModels(): AppState {
  return run(initialState(), {
    type: "setModels",
    models: [
      model("m1", "sdxl", { lastUsed: 10 }),
      model("m2", "sdxl_pony", { lastUsed: 20 }),
      model("flux", "flux1_dev"),
      model("edit", "qwen_image_edit_2511", { modes: ["edit"], isEditModel: true }),
    ],
  });
}

describe("reducer", () => {
  it("picks the most recently used text-to-image model", () => {
    const s = withModels();
    expect(s.create.modelId).toBe("m2");
  });

  it("moving a dial clears the matching Fine-tune override", () => {
    let s = run(withModels(), { type: "setFineTune", patch: { steps: 50, cfg: 9, width: 800, height: 600, seed: 5 } });
    s = run(s, { type: "setDial", dial: "quality", value: "best" });
    expect(s.create.fineTune.steps).toBeUndefined();
    expect(s.create.fineTune.cfg).toBe(9);
    s = run(s, { type: "setDial", dial: "stick", value: 0.2 });
    expect(s.create.fineTune.cfg).toBeUndefined();
    s = run(s, { type: "setDial", dial: "shape", value: "wide" });
    expect(s.create.fineTune.width).toBeUndefined();
    expect(s.create.fineTune.seed).toBe(5);
  });

  it("resetting a Fine-tune field removes it", () => {
    let s = run(withModels(), { type: "setFineTune", patch: { steps: 50 } });
    s = run(s, { type: "setFineTune", patch: { steps: null } });
    expect("steps" in s.create.fineTune).toBe(false);
  });

  it("switching family drops family-specific overrides but keeps text and seed", () => {
    let s = run(withModels(), { type: "selectModel", modelId: "m1" });
    s = run(s, { type: "setFineTune", patch: { sampler: "euler", negativePrompt: SENTINEL, seed: 3 } }, { type: "setDial", dial: "stick", value: 0.9 });
    s = run(s, { type: "selectModel", modelId: "flux" });
    expect(s.create.fineTune).toEqual({ negativePrompt: SENTINEL, seed: 3 });
    expect(s.create.stick).toBeNull();
  });

  it("Keep this look locks and follows the selected result's seed", () => {
    let s = run(withModels(), { type: "addResults", batch: null, images: [result("a", 11), result("b", 12)], refs: [ref("a"), ref("b")] });
    expect(s.selectedResultId).toBe("a");
    s = run(s, { type: "keepLook", on: true });
    expect(s.create.fineTune.seed).toBe(11);
    s = run(s, { type: "selectResult", id: "b" });
    expect(s.create.fineTune.seed).toBe(12);
    s = run(s, { type: "keepLook", on: false });
    expect(s.create.fineTune.seed).toBeUndefined();
    // A hand-typed seed does not follow the selection.
    s = run(s, { type: "setFineTune", patch: { seed: 999 } }, { type: "selectResult", id: "a" });
    expect(s.create.fineTune.seed).toBe(999);
  });

  it("keeps images that are still shown somewhere and drops the rest", () => {
    let s = run(withModels(), { type: "addResults", batch: null, images: [result("a", 1)], refs: [ref("a")] });
    s = run(s, { type: "editLoad", ref: ref("a") });
    s = run(s, { type: "removeResult", id: "a" });
    expect(Object.keys(s.images)).toEqual(["a"]); // still the edit original
    s = run(s, { type: "editClear" });
    expect(Object.keys(s.images)).toEqual([]);
  });

  it("edit chain: push, undo, redo, branch", () => {
    let s = run(withModels(), { type: "editLoad", ref: ref("o") }, { type: "editPush", ref: ref("e1") }, { type: "editPush", ref: ref("e2") });
    expect(s.edit.chain.map((n) => n.label)).toEqual(["Original", "Edit 1", "Edit 2"]);
    expect(s.edit.index).toBe(2);
    s = run(s, { type: "editGoto", index: s.edit.index - 1 });
    expect(s.edit.index).toBe(1);
    s = run(s, { type: "editGoto", index: 99 });
    expect(s.edit.index).toBe(2);
    s = run(s, { type: "editGoto", index: 0 }, { type: "editPush", ref: ref("b1") });
    expect(s.edit.chain.map((n) => n.imageId)).toEqual(["o", "b1"]);
    expect(Object.keys(s.images).sort()).toEqual(["b1", "o"]);
  });

  it("edit chain: delete an edit", () => {
    const base = run(withModels(), { type: "editLoad", ref: ref("o") }, { type: "editPush", ref: ref("e1") }, { type: "editPush", ref: ref("e2") }, { type: "editPush", ref: ref("e3") });
    // Deleting the shown edit steps back to the one before it and frees its image.
    let s = run(base, { type: "editDelete", index: 3 });
    expect(s.edit.chain.map((n) => n.imageId)).toEqual(["o", "e1", "e2"]);
    expect(s.edit.index).toBe(2);
    expect(Object.keys(s.images).sort()).toEqual(["e1", "e2", "o"]);
    // Deleting a middle edit renumbers and keeps the same image selected.
    s = run(base, { type: "editDelete", index: 1 });
    expect(s.edit.chain.map((n) => [n.imageId, n.label])).toEqual([["o", "Original"], ["e2", "Edit 1"], ["e3", "Edit 2"]]);
    expect(s.edit.chain[s.edit.index].imageId).toBe("e3");
    // Deleting the shown middle edit shows the edit that took its place, not the original.
    s = run(base, { type: "editGoto", index: 1 }, { type: "editDelete", index: 1 });
    expect(s.edit.chain[s.edit.index].imageId).toBe("e2");
    // The original cannot be deleted.
    expect(run(base, { type: "editDelete", index: 0 })).toBe(base);
  });

  it("Reset clears prompts, results, edit chain and description", () => {
    let s = withModels();
    s = run(
      s,
      { type: "patchCreate", patch: { prompt: SENTINEL } },
      { type: "setFineTune", patch: { negativePrompt: SENTINEL, seed: 4, vaeTiling: true } },
      { type: "addResults", batch: { id: "b", request: { prompt: SENTINEL } as never }, images: [result("a", 1)], refs: [ref("a")] },
      { type: "editLoad", ref: ref("x") },
      { type: "patchEdit", patch: { instruction: SENTINEL, restylePrompt: SENTINEL } },
      { type: "describeLoad", ref: ref("d") },
      { type: "patchDescribe", patch: { text: SENTINEL } },
    );
    expect(JSON.stringify(s)).toContain(SENTINEL);
    const cleared = run(s, { type: "clearSession" });
    expect(JSON.stringify(cleared)).not.toContain(SENTINEL);
    expect(cleared.results).toEqual([]);
    expect(cleared.images).toEqual({});
    expect(cleared.edit.chain).toEqual([]);
    expect(cleared.describe.imageId).toBeNull();
    expect(cleared.create.modelId).toBe(s.create.modelId);
    expect(cleared.create.fineTune).toEqual({ vaeTiling: true });
    expect(cleared.sessionNonce).toBe(s.sessionNonce + 1);
    expect(referencedImageIds(cleared).size).toBe(0);
  });
});

describe("requests", () => {
  it("builds a txt2img request with family defaults and drops unusable overrides", () => {
    let s = run(withModels(), { type: "selectModel", modelId: "flux" }, { type: "patchCreate", patch: { prompt: " a fox " } });
    s = run(s, { type: "setFineTune", patch: { negativePrompt: "blurry", steps: 12 } });
    const req = buildCreateRequest(s.create, {
      ui: FAMILY_UI.flux1_dev,
      loras: [lora("sd15lora", "sd15"), lora("fluxlora", "flux1_dev")],
      model: s.models!.find((m) => m.id === "flux")!,
      settings: null,
    });
    expect(req.prompt).toBe("a fox");
    expect(req.fineTune).toEqual({ steps: 12 }); // Flux has no negative prompt
    expect(req.dials.stick).toBeCloseTo(0.5);
    expect(req.mode).toBe("txt2img");
    expect(req.addTriggerWords).toBe(true);
  });

  it("drops numbers Rust can't deserialize (u32 steps/size, integer seed/clip skip)", () => {
    const create = { ...withModels().create, prompt: "p", fineTune: { steps: -5, width: 832.5, height: 1216, clipSkip: 1.5, seed: -1, cfg: 4.5 } };
    const req = buildCreateRequest(create, { ui: FAMILY_UI.sdxl, loras: [], model: model("m1", "sdxl"), settings: null });
    expect(req.fineTune).toEqual({ height: 1216, seed: -1, cfg: 4.5 });
    expect(presetFromCreate("P", create, { model: null, loras: [] }).fineTune).toEqual({ height: 1216, seed: -1, cfg: 4.5 });
  });

  it("filters LoRAs that don't match the model's architecture", () => {
    let s = run(withModels(), { type: "selectModel", modelId: "m1" });
    s = run(s, { type: "patchCreate", patch: { loras: [{ loraId: "p", weight: 0.8 }, { loraId: "x", weight: 1 }, { loraId: "gone", weight: 1 }] } });
    const req = buildCreateRequest(s.create, {
      ui: FAMILY_UI.sdxl,
      loras: [lora("p", "sdxl_pony"), lora("x", "sd15")],
      model: s.models!.find((m) => m.id === "m1")!,
      settings: null,
    });
    expect(req.loras).toEqual([{ loraId: "p", weight: 0.8 }]);
  });

  it("variations drop the seed only", () => {
    const req = buildCreateRequest(
      { ...withModels().create, prompt: "p", fineTune: { seed: 5, steps: 9 } },
      { ui: FAMILY_UI.sdxl, loras: [], model: model("m1", "sdxl"), settings: null },
    );
    expect(variationRequest(req).fineTune).toEqual({ steps: 9 });
    expect(variationRequest(req).prompt).toBe("p");
  });

  it("builds instruction-edit and restyle requests", () => {
    const e = { ...withModels().edit, instruction: "make it blue", restylePrompt: "oil painting", stayClose: 0.8, change: "strong" as const };
    const ins = buildEditRequest(e, {
      mode: "instruction",
      source: ref("src"),
      model: model("edit", "qwen_image_edit_2511"),
      ui: FAMILY_UI.qwen_image_edit_2511,
      maskImageId: "mask",
      size: [1024, 768],
    });
    expect(ins).toMatchObject({ mode: "edit", prompt: "make it blue", refImageIds: ["src"], maskImageId: "mask", fineTune: { width: 1024, height: 768 } });
    expect(ins.dials.stick).toBe(0.8);
    const rs = buildEditRequest(e, { mode: "restyle", source: ref("src"), model: model("m1", "sdxl"), ui: FAMILY_UI.sdxl, maskImageId: null, size: [512, 512] });
    expect(rs).toMatchObject({ mode: "img2img", prompt: "oil painting", initImageId: "src", strength: 0.75 });
  });

  it("fits edit sizes to ~1 MP in multiples of 16", () => {
    expect(fitEditSize(1024, 1024)).toEqual([1024, 1024]);
    expect(fitEditSize(4000, 3000)).toEqual([1184, 880]);
    expect(fitEditSize(300, 200)).toEqual([304, 256]);
    for (const n of fitEditSize(3000, 1999)) expect(n % 16).toBe(0);
    for (const n of fitEditSize(3000, 1999, 1024 * 1024, 64)) expect(n % 64).toBe(0);
    const sizes = (w: number, h: number) => (["smaller", "normal", "larger"] as const).map((c) => editOutputSize(w, h, c, 16));
    // Every choice must give a different size, including for sources at or below 1 MP.
    for (const [w, h] of [[1024, 1024], [512, 512], [832, 1216], [4000, 3000]] as const) {
      const [s, n, l] = sizes(w, h);
      expect(s[0] * s[1]).toBeLessThan(n[0] * n[1]);
      expect(n[0] * n[1]).toBeLessThan(l[0] * l[1]);
    }
    expect(sizes(1024, 1024)).toEqual([[768, 768], [1024, 1024], [1280, 1280]]);
  });
});

describe("presets", () => {
  it("never stores the prompt or the negative prompt", () => {
    const s = run(
      withModels(),
      { type: "patchCreate", patch: { prompt: SENTINEL, loras: [{ loraId: "l1", weight: 0.7 }] } },
      { type: "setFineTune", patch: { negativePrompt: SENTINEL, steps: 33, hires: true, hiresScale: 1.5, hiresDenoise: 0.4 } },
    );
    const p = presetFromCreate("My look", s.create, { model: s.models![1], loras: [lora("l1", "sdxl_pony", 77)] });
    expect(JSON.stringify(p)).not.toContain(SENTINEL);
    expect(p.fineTune).toEqual({ steps: 33, hires: true });
    expect(p.loras).toEqual([{ loraId: "l1", civitaiVersionId: 77, name: "l1", weight: 0.7 }]);
    expect(p.family).toBe("sdxl_pony");
  });

  it("applies a preset and keeps the user's text", () => {
    const s = run(withModels(), { type: "patchCreate", patch: { prompt: "keep me" } }, { type: "setFineTune", patch: { negativePrompt: "neg" } });
    const preset = presetFromCreate("P", { ...s.create, shape: "portrait", fineTune: { steps: 40 }, loras: [{ loraId: "l1", weight: 0.5 }] }, {
      model: s.models![0],
      loras: [lora("l1", "sdxl", 5)],
      id: "p1",
    });
    const res = applyPreset(preset, s.create, { models: s.models!, loras: [lora("other-id", "sdxl", 5)], styleIds: [] });
    expect(res.patch.modelId).toBe("m1");
    expect(res.patch.shape).toBe("portrait");
    expect(res.patch.fineTune).toEqual({ steps: 40, negativePrompt: "neg" });
    expect(res.patch.loras).toEqual([{ loraId: "other-id", weight: 0.5 }]); // matched by CivitAI version
    expect(res.missingModel).toBeNull();
    expect("prompt" in res.patch).toBe(false);
  });

  it("reports a missing model and LoRAs", () => {
    const s = withModels();
    const res = applyPreset(
      {
        id: "x",
        name: "X",
        family: "z_image_turbo",
        modelId: "zit",
        civitaiVersionId: 123,
        styleId: "gone",
        shape: null,
        quality: null,
        stick: null,
        count: null,
        fineTune: {},
        loras: [{ loraId: null, civitaiVersionId: 9, name: "Thing", weight: 1 }],
        builtin: true,
      },
      s.create,
      { models: s.models!, loras: [], styleIds: [] },
    );
    expect(res.missingModel).toEqual({ civitaiVersionId: 123, family: "z_image_turbo" });
    expect(res.missingLoras).toHaveLength(1);
    expect(res.missingStyle).toBe(true);
    expect(res.patch.modelId).toBeUndefined();
  });

  it("family-only presets keep a matching current model", () => {
    const s = withModels(); // current m2 (sdxl_pony)
    const base = { id: "b", name: "B", modelId: null, civitaiVersionId: null, styleId: null, shape: null, quality: "best" as const, stick: null, count: null, fineTune: {}, loras: [], builtin: true };
    expect(applyPreset({ ...base, family: "sdxl_pony" }, s.create, { models: s.models!, loras: [], styleIds: [] }).patch.modelId).toBe("m2");
    expect(applyPreset({ ...base, family: "sdxl" }, s.create, { models: s.models!, loras: [], styleIds: [] }).patch.modelId).toBe("m1");
    expect(applyPreset({ ...base, family: null }, s.create, { models: s.models!, loras: [], styleIds: [] }).patch.modelId).toBe("m2");
  });
});

describe("choosing None", () => {
  const base: Preset = { id: "b", name: "B", family: null, modelId: null, civitaiVersionId: null, styleId: null, shape: "portrait", quality: "best" as const, stick: 0.8, count: 4 as const, fineTune: { steps: 50 }, loras: [], builtin: true };
  const opts = (s: ReturnType<typeof withModels>) => ({ models: s.models!, loras: [], styleIds: [] });
  const apply = (s: ReturnType<typeof withModels>, p = base) => run(s, { type: "patchCreate", patch: applyPreset(p, s.create, opts(s)).patch });
  const none = (s: ReturnType<typeof withModels>) => run(s, { type: "patchCreate", patch: clearPreset(s.create, opts(s)) });

  it("clears the preset and restores what it overrode, keeping the user's text", () => {
    let s = run(withModels(), { type: "patchCreate", patch: { prompt: "keep me", shape: "wide", quality: "fast" } }, { type: "setFineTune", patch: { steps: 12, negativePrompt: "neg" } });
    const before = s.create;
    s = apply(s);
    expect(s.create.presetId).toBe("b");
    expect(s.create.shape).toBe("portrait");
    s = run(s, { type: "setFineTune", patch: { negativePrompt: "neg2" } });
    s = none(s);
    expect(s.create).toEqual({ ...before, fineTune: { steps: 12, negativePrompt: "neg2" } });
  });

  it("keeps edits made after the preset was applied", () => {
    let s = apply(run(withModels(), { type: "patchCreate", patch: { shape: "wide" } }));
    s = run(s, { type: "patchCreate", patch: { shape: "square" } }, { type: "setFineTune", patch: { seed: 7 } });
    s = none(s);
    expect(s.create.shape).toBe("square");
    expect(s.create.quality).toBe("balanced");
    expect(s.create.presetId).toBeNull();
  });

  it("restores the original settings after hopping between presets", () => {
    let s = apply(run(withModels(), { type: "patchCreate", patch: { shape: "wide" } }));
    s = apply(s, { ...base, id: "c", shape: "square" });
    s = none(s);
    expect(s.create.shape).toBe("wide");
    expect(s.create.presetBase).toBeNull();
  });

  it("does not bring back a deleted style or model", () => {
    let s = run(withModels(), { type: "patchCreate", patch: { styleId: "gone" } });
    s = apply(s);
    expect(clearPreset(s.create, opts(s)).styleId).toBeNull();
  });

  it("changing the model drops the preset snapshot", () => {
    const s = run(apply(withModels()), { type: "selectModel", modelId: "m1" });
    expect(s.create.presetBase).toBeNull();
    expect(s.create.presetId).toBeNull();
  });

  it("dropping presetId through a plain patch (paste) drops the snapshot too", () => {
    const s = run(apply(withModels()), { type: "patchCreate", patch: { presetId: null } });
    expect(s.create.presetBase).toBeNull();
  });

  it("deleting the active preset drops the snapshot", () => {
    const s = run(apply(withModels()), { type: "setPresets", presets: [] });
    expect(s.create.presetId).toBeNull();
    expect(s.create.presetBase).toBeNull();
  });
});

describe("settingsSummary", () => {
  it("never mentions the prompt and shows the key numbers", () => {
    expect(settingsSummary(result("a", 42))).toBe("Model One · 1024×1024 · 30 steps · CFG 6 · dpm++2m karras · seed 42");
    expect(settingsSummary(result("a", 1, { guidance: 3.5, cfg: 1, sampler: "euler", scheduler: null }))).toBe(
      "Model One · 1024×1024 · 30 steps · guidance 3.5 · euler · seed 1",
    );
  });

  it("labels upscales instead of repeating the source's sampling settings", () => {
    expect(settingsSummary(result("u", 42, { kind: "upscaled", width: 2048, height: 2048 }))).toBe("Upscaled · 2048×2048 · from Model One · seed 42");
    // Upscale of an imported image: Rust sends an empty model id and seed 0.
    expect(settingsSummary(result("u", 0, { kind: "upscaled", modelId: "", modelLabel: "Upscaled image", steps: 0, cfg: 0, sampler: null, scheduler: null }))).toBe(
      "Upscaled · 1024×1024",
    );
  });
});
