import { describe, expect, it } from "vitest";
import { galleryGenerationText, gallerySettingsSummary } from "./fromGallery";
import { parseGenerationData } from "./parse";

// Shape of a live /model-versions/{id} image `meta` (2026-09-28), text made up.
const LIVE_META = {
  VAE: "vae-ft-mse-840000-ema-pruned.safetensors",
  Size: "512x768",
  seed: 3277121308,
  Model: "RVHYPO",
  steps: 6,
  hashes: { vae: "e9ed949371", model: "0928b30687" },
  prompt: "a lighthouse on a cliff at dusk, (warm light:1.2), mist",
  sampler: "DPM++ SDE Karras",
  cfgScale: 1.5,
  resources: [{ hash: "0928b30687", name: "RVHYPO", type: "model" }],
  "Model hash": "0928b30687",
  "Hires steps": "2",
  "Hires upscale": "2",
  "Hires upscaler": "4x_NMKD-Superscale-SP_178000_G",
  negativePrompt: "(deformed, distorted:1.3), blurry, text",
  "Denoising strength": "0.35",
};

const SOURCE = { type: "Checkpoint", versionId: 501240, modelName: "Realistic Vision", versionName: "V5.1 Hyper" };

describe("galleryGenerationText", () => {
  it("round-trips live meta through the paste parser", () => {
    const text = galleryGenerationText(LIVE_META, SOURCE)!;
    const p = parseGenerationData(text)!;
    expect(p.prompt).toBe(LIVE_META.prompt);
    expect(p.negative).toBe(LIVE_META.negativePrompt);
    expect(p.steps).toBe(6);
    expect(p.cfg).toBe(1.5);
    expect(p.sampler).toBe("DPM++ SDE Karras");
    expect(p.seed).toBe(3277121308);
    expect([p.width, p.height]).toEqual([512, 768]);
    expect(p.denoise).toBe(0.35);
    expect(p.hires?.scale).toBe(2);
    expect(p.modelHash).toBe("0928b30687");
  });

  it("puts this page's checkpoint first so Create selects or offers it", () => {
    const p = parseGenerationData(galleryGenerationText(LIVE_META, SOURCE)!)!;
    const ck = p.resources.filter((r) => r.type === "checkpoint");
    expect(ck).toHaveLength(1);
    expect(ck[0].modelVersionId).toBe(501240);
  });

  it("adds a LoRA page's LoRA next to the image's own resources", () => {
    const meta = {
      prompt: "paper cut style, a fox in a forest",
      steps: 20,
      cfgScale: 3.5,
      civitaiResources: [{ type: "checkpoint", modelVersionId: 691639, modelName: "FLUX" }],
    };
    const p = parseGenerationData(galleryGenerationText(meta, { type: "LORA", versionId: 77, modelName: "Paper Cut", versionName: "v1" })!)!;
    expect(p.resources.find((r) => r.type === "checkpoint")?.modelVersionId).toBe(691639);
    expect(p.resources.find((r) => r.type === "lora")).toMatchObject({ modelVersionId: 77, weight: 1 });
  });

  it("doesn't add a LoRA page's LoRA twice when the image names it by hash", () => {
    const meta = {
      prompt: "paper cut style, a fox <lora:papercut_v1:0.7>",
      steps: 20,
      cfgScale: 5,
      resources: [{ type: "lora", name: "papercut_v1", hash: "1122334455", weight: 0.7 }],
      "Lora hashes": "papercut_v1: 1122334455",
    };
    const p = parseGenerationData(galleryGenerationText(meta, { type: "LORA", versionId: 77, modelName: "Paper Cut", versionName: "v1" })!)!;
    const loras = p.resources.filter((r) => r.type === "lora");
    expect(loras).toHaveLength(1);
    expect(loras[0].hash?.toLowerCase()).toBe("1122334455");
  });

  it("keeps commas inside values", () => {
    const p = parseGenerationData(galleryGenerationText({ prompt: "x", steps: 10, Model: "Juggernaut, v9", seed: 1 }, null)!)!;
    expect(p.modelName).toBe("Juggernaut, v9");
    expect(p.seed).toBe(1);
  });

  it("returns null when there is nothing to apply", () => {
    expect(galleryGenerationText(null, SOURCE)).toBeNull();
    expect(galleryGenerationText({ Model: "x" }, null)).toBeNull();
  });

  it("summarises settings", () => {
    expect(gallerySettingsSummary(LIVE_META)).toEqual(["6 steps", "CFG 1.5", "DPM++ SDE Karras", "512×768", "Seed 3277121308"]);
  });
});
