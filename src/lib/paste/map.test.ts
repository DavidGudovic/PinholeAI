import { describe, expect, it } from "vitest";
import { FAMILY_UI } from "../state/familyFixtures";
import {
  SD_SAMPLERS,
  SD_SCHEDULERS,
  defaultStickPosition,
  mapSampler,
  planPaste,
  promptHasPrefix,
  shapeFor,
  stickPositionFor,
  stickValue,
} from "./map";
import { parseGenerationData } from "./parse";
import { A1111_HIRES, COMFY_ISH, FLUX_CIVITAI, FLUX_FORGE, ILLUSTRIOUS_CIVITAI, PONY_A1111 } from "./samples";

describe("mapSampler", () => {
  const cases: [string | null, string | null, string | null, string | null, boolean][] = [
    // raw sampler, raw scheduler, → sampler, scheduler, exact
    ["DPM++ 2M Karras", null, "dpm++2m", "karras", true],
    ["DPM++ 2M", "Karras", "dpm++2m", "karras", true],
    ["Euler a", null, "euler_a", null, true],
    ["Euler a", "Automatic", "euler_a", null, true],
    ["Euler", "Simple", "euler", "simple", true],
    ["DPM++ 2M SDE Karras", null, "dpm++2m_sde", "karras", true],
    ["DPM++ 2M SDE Exponential", null, "dpm++2m_sde", "exponential", true],
    ["DPM++ SDE Karras", null, "dpm++2m_sde", "karras", false],
    ["DPM++ 2S a", null, "dpm++2s_a", null, true],
    ["DPM2 a Karras", null, "dpm++2s_a", "karras", false],
    ["DDIM", null, "ddim_trailing", null, false],
    ["LCM", null, "lcm", null, true],
    ["LMS Karras", null, "lms", "karras", true],
    ["Heun", null, "heun", null, true],
    ["UniPC", null, null, null, false],
    ["Restart", null, null, null, false],
    ["DPM++ 2M", "SGM Uniform", "dpm++2m", "sgm_uniform", true],
    ["DPM++ 2M", "Align Your Steps", "dpm++2m", "ays", true],
    ["DPM++ 2M", "Normal", "dpm++2m", "discrete", true],
    ["DPM++ 2M", "Polyexponential", "dpm++2m", "exponential", false],
    // Comfy names
    ["euler_ancestral", "normal", "euler_a", "discrete", true],
    ["dpmpp_2m", "karras", "dpm++2m", "karras", true],
    ["dpmpp_2m_sde_gpu", "karras", "dpm++2m_sde", "karras", true],
    ["dpmpp_2m_karras", null, "dpm++2m", "karras", true],
    ["dpmpp_3m_sde_gpu", "exponential", "dpm++2m_sde", "exponential", false],
    ["uni_pc_bh2", "simple", null, "simple", false],
    ["res_multistep", "beta", "res_multistep", "beta", true],
    ["er_sde", "kl_optimal", "er_sde", "kl_optimal", true],
    ["euler_cfg_pp", null, "euler_cfg_pp", null, true],
    ["gradient_estimation", null, "euler_ge", null, true],
    ["k_euler_a", null, "euler_a", null, true],
    // CivitAI placeholders
    ["Undefined", null, null, null, true],
    [null, null, null, null, true],
    ["Totally New Sampler", null, null, null, false],
  ];
  it.each(cases)("%s / %s → %s + %s", (s, sch, es, esch, exact) => {
    const m = mapSampler(s, sch);
    expect(m.sampler).toBe(es);
    expect(m.scheduler).toBe(esch);
    expect(m.exact).toBe(exact);
    if (!exact) expect(m.notes.length).toBeGreaterThan(0);
  });

  it("only produces names sd.cpp knows", () => {
    const samplers = new Set(SD_SAMPLERS.map((s) => s.id));
    const schedulers = new Set(SD_SCHEDULERS.map((s) => s.id));
    for (const [s, sch] of cases) {
      const m = mapSampler(s, sch);
      if (m.sampler) expect(samplers.has(m.sampler)).toBe(true);
      if (m.scheduler) expect(schedulers.has(m.scheduler)).toBe(true);
    }
  });
});

describe("dial math", () => {
  it("maps the registry default into a dial position and back", () => {
    const ui = FAMILY_UI.sd15; // cfg 4…11, default 7
    const pos = defaultStickPosition(ui);
    expect(pos).toBeCloseTo(3 / 7);
    expect(stickValue(ui, pos)).toBe(7);
    expect(stickPositionFor(ui, 100)).toBe(1);
    expect(stickPositionFor(ui, 0)).toBe(0);
  });
  it("falls back to the middle for degenerate ranges", () => {
    expect(defaultStickPosition(FAMILY_UI.z_image_turbo)).toBe(0.5);
    expect(defaultStickPosition(null)).toBe(0.5);
  });
  it("picks shapes by exact size or closest aspect", () => {
    expect(shapeFor(FAMILY_UI.sdxl, 832, 1216)).toEqual({ shape: "portrait", exact: true });
    expect(shapeFor(FAMILY_UI.sdxl, 896, 1152)).toEqual({ shape: "portrait", exact: false });
    expect(shapeFor(FAMILY_UI.sdxl, 1920, 1080)).toEqual({ shape: "wide", exact: false });
    expect(shapeFor(FAMILY_UI.sd15, 512, 512)).toEqual({ shape: "square", exact: true });
  });
  it("detects an automatic prefix already in the prompt", () => {
    expect(promptHasPrefix("score_9, score_8_up, score_7_up, 1girl", "score_9, score_8_up, score_7_up, ")).toBe(true);
    expect(promptHasPrefix("1girl, score_9", "score_9, score_8_up, score_7_up, ")).toBe(false);
  });
});

describe("planPaste", () => {
  it("applies a Pony sample to a Pony model", () => {
    const plan = planPaste(parseGenerationData(PONY_A1111)!, FAMILY_UI.sdxl_pony);
    expect(plan.prompt).toMatch(/^score_9/);
    expect(plan.fineTune).toMatchObject({
      negativePrompt: "score_4, score_5, score_6, lowres, bad anatomy,\nworst quality",
      steps: 30,
      sampler: "dpm++2m",
      scheduler: "karras",
      cfg: 7,
      seed: 3141592653,
      clipSkip: 2,
      autoPromptPrefix: false,
    });
    expect(plan.fineTune.width).toBeUndefined(); // 832x1216 is the Portrait chip
    expect(plan.shape).toBe("portrait");
    expect(plan.keepLook).toBe(true);
    expect(plan.stick).toBeCloseTo(0.75); // 7 in 4…8
    expect(plan.applied.some((a) => a.includes("outside"))).toBe(false);
  });

  it("maps Flux CFG to guidance when there is no explicit guidance", () => {
    const plan = planPaste(parseGenerationData(FLUX_CIVITAI)!, FAMILY_UI.flux1_dev);
    expect(plan.fineTune.guidance).toBe(3.5);
    expect(plan.fineTune.cfg).toBeUndefined();
    expect(plan.fineTune.negativePrompt).toBeUndefined();
    expect(plan.fineTune.sampler).toBeUndefined(); // "Undefined"
    expect(plan.fineTune.clipSkip).toBeUndefined(); // Flux has no clip skip; 1 is not worth a note
    expect(plan.stick).toBeCloseTo(0.5);
    expect(plan.skipped.map((s) => s.what)).toContain("CFG 3.5");
  });

  it("uses explicit Flux guidance and keeps quiet about CFG 1", () => {
    const plan = planPaste(parseGenerationData(FLUX_FORGE)!, FAMILY_UI.flux1_dev);
    expect(plan.fineTune).toMatchObject({ guidance: 3.5, sampler: "euler", scheduler: "simple", seed: 42, width: 896, height: 1152 });
    expect(plan.skipped.find((s) => s.what.startsWith("CFG"))).toBeUndefined();
    expect(plan.ignoredKeys).toEqual([]); // Version / Module N are quiet
  });

  it("skips what a fixed-CFG family can't use", () => {
    const plan = planPaste(parseGenerationData(PONY_A1111)!, FAMILY_UI.z_image_turbo);
    expect(plan.fineTune.cfg).toBeUndefined();
    expect(plan.fineTune.negativePrompt).toBeUndefined();
    expect(plan.fineTune.clipSkip).toBeUndefined();
    const whats = plan.skipped.map((s) => s.what);
    expect(whats).toContain("Negative prompt");
    expect(whats).toContain("CFG 7");
    expect(whats).toContain("Clip skip 2");
  });

  it("applies hires fix and reports what it could not use", () => {
    const plan = planPaste(parseGenerationData(A1111_HIRES)!, FAMILY_UI.sd15);
    expect(plan.fineTune).toMatchObject({ hires: true, hiresScale: 2, hiresDenoise: 0.45, sampler: "dpm++2m", scheduler: "karras", cfg: 6 });
    expect(plan.shape).toBe("portrait");
    const whats = plan.skipped.map((s) => s.what);
    expect(whats).toContain("Hires steps 10");
    expect(whats).toContain('Upscaler "R-ESRGAN 4x+"');
    expect(whats).toContain("ADetailer (face fix)");
    // ADetailer values (which contain prompt text) never show up as key names to list.
    expect(plan.ignoredKeys.some((k) => k.startsWith("ADetailer"))).toBe(false);
  });

  it("marks a non-chip size as custom", () => {
    const plan = planPaste(parseGenerationData("x\nSteps: 5, Seed: 1, Size: 1000x1500")!, FAMILY_UI.sdxl);
    expect(plan.fineTune.width).toBe(1000);
    expect(plan.fineTune.height).toBe(1504);
    expect(plan.shape).toBe("portrait");
  });

  it("works without a model (no family checks)", () => {
    const plan = planPaste(parseGenerationData(COMFY_ISH)!, null);
    expect(plan.fineTune).toMatchObject({ cfg: 6.5, steps: 30, sampler: "dpm++2m_sde", scheduler: "karras", negativePrompt: "blurry" });
    expect(plan.stick).toBeNull();
  });

  it("lists unknown settings by name only", () => {
    const plan = planPaste(parseGenerationData(ILLUSTRIOUS_CIVITAI)!, FAMILY_UI.sdxl_illustrious);
    expect(plan.ignoredKeys).toEqual([]); // workflow / draft / Created Date / Civitai metadata are quiet
    expect(plan.fineTune.autoPromptPrefix).toBe(false); // "masterpiece, best quality" already there
    const p2 = planPaste(parseGenerationData("x\nSteps: 5, Seed: 1, Face restoration: CodeFormer")!, FAMILY_UI.sdxl);
    expect(p2.ignoredKeys).toEqual(["Face restoration"]);
  });
});
