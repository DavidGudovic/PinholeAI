import { describe, expect, it } from "vitest";
import { looksLikeGenerationData, parseGenerationData, tokenizeSettings } from "./parse";
import { A1111_HIRES, COMFY_ISH, FLUX_CIVITAI, FLUX_FORGE, ILLUSTRIOUS_CIVITAI, PONY_A1111 } from "./samples";

describe("parseGenerationData", () => {
  it("parses an A1111 Pony sample with LoRA tags, hashes and a multi-line negative", () => {
    const p = parseGenerationData(PONY_A1111)!;
    expect(p.hasSettings).toBe(true);
    expect(p.prompt).toBe(
      "score_9, score_8_up, score_7_up, 1girl, solo, looking at viewer, cherry blossoms\nupper body, smile",
    );
    expect(p.negative).toBe("score_4, score_5, score_6, lowres, bad anatomy,\nworst quality");
    expect(p.steps).toBe(30);
    expect(p.sampler).toBe("DPM++ 2M Karras");
    expect(p.scheduler).toBeNull();
    expect(p.cfg).toBe(7);
    expect(p.seed).toBe(3141592653);
    expect([p.width, p.height]).toEqual([832, 1216]);
    expect(p.clipSkip).toBe(2);
    expect(p.modelName).toBe("ponyDiffusionV6XL_v6StartWithThisOne");
    expect(p.modelHash).toBe("67ab2fd8ec");
    expect(p.loraTags).toEqual([
      { name: "add_detail", weight: 0.6 },
      { name: "Pony_Style_XL", weight: 0.8 },
    ]);
    const ckpt = p.resources.find((r) => r.type === "checkpoint")!;
    expect(ckpt).toMatchObject({ hash: "67ab2fd8ec", modelName: "ponyDiffusionV6XL_v6StartWithThisOne", modelVersionId: null });
    const loras = p.resources.filter((r) => r.type === "lora");
    expect(loras).toEqual([
      { type: "lora", modelVersionId: null, modelName: "add_detail", modelVersionName: null, hash: "7c6bad76eb54", weight: 0.6 },
      { type: "lora", modelVersionId: null, modelName: "Pony_Style_XL", modelVersionName: null, hash: "0123abcd4567", weight: 0.8 },
    ]);
    expect(p.resources.find((r) => r.type === "vae")?.hash).toBe("235745af8d");
    expect(p.extra["Version"]).toBe("v1.7.0");
  });

  it("parses CivitAI on-site data with a JSON resource list containing commas", () => {
    const p = parseGenerationData(ILLUSTRIOUS_CIVITAI)!;
    expect(p.prompt).toBe("masterpiece, best quality, 1girl, red hair, forest, dappled sunlight");
    expect(p.negative).toBe("worst quality, low quality, watermark");
    expect(p.cfg).toBe(5.5);
    expect(p.sampler).toBe("Euler a");
    expect(p.seed).toBe(987654321);
    expect(p.resources).toEqual([
      { type: "checkpoint", modelVersionId: 1190596, modelName: "WAI-NSFW-illustrious-SDXL", modelVersionName: "v11.0", hash: null, weight: null },
      { type: "lora", modelVersionId: 456789, modelName: "Detailer, Illustrious", modelVersionName: "v1.0", hash: null, weight: 0.8 },
      { type: "embed", modelVersionId: 222, modelName: "lazyneg", modelVersionName: "v1", hash: null, weight: 1 },
    ]);
    expect(p.extra["workflow"]).toBe("txt2img");
    expect(p.extra["draft"]).toBe("false");
    expect(p.extra["Created Date"]).toBe("2024-11-02T10:11:12.1234567Z");
    expect(p.extra["Civitai metadata"]).toBe('{"remixOfId":12345}');
  });

  it("parses Flux data without a negative prompt", () => {
    const p = parseGenerationData(FLUX_CIVITAI)!;
    expect(p.negative).toBeNull();
    expect(p.prompt).toMatch(/^A cinematic photo/);
    expect(p.cfg).toBe(3.5);
    expect(p.guidance).toBeNull();
    expect(p.sampler).toBe("Undefined");
    expect(p.resources).toHaveLength(1);
    expect(p.resources[0]).toMatchObject({ type: "checkpoint", modelVersionId: 691639 });
  });

  it("reads Forge's Distilled CFG Scale as guidance", () => {
    const p = parseGenerationData(FLUX_FORGE)!;
    expect(p.cfg).toBe(1);
    expect(p.guidance).toBe(3.5);
    expect(p.scheduler).toBe("Simple");
    expect(p.extra["Module 1"]).toBe("ae");
    expect(p.resources[0]).toMatchObject({ type: "checkpoint", hash: "d5d1e2a8b0", modelName: "flux1-dev-Q8_0" });
  });

  it("parses Schedule type, hires fix and ADetailer keys", () => {
    const p = parseGenerationData(A1111_HIRES)!;
    expect(p.negative).toBe("(worst quality:1.4), cartoon");
    expect(p.sampler).toBe("DPM++ 2M");
    expect(p.scheduler).toBe("Karras");
    expect(p.denoise).toBe(0.45);
    expect(p.hires).toEqual({ scale: 2, steps: 10, upscaler: "R-ESRGAN 4x+" });
    expect(p.extra["ADetailer prompt"]).toBe("detailed face, smile");
    expect(p.extra["ADetailer model"]).toBe("face_yolov8n.pt");
  });

  it("parses Comfy-style sampler names", () => {
    const p = parseGenerationData(COMFY_ISH)!;
    expect(p.sampler).toBe("dpmpp_2m_sde_gpu");
    expect(p.scheduler).toBe("karras");
    expect(p.denoise).toBe(1);
    expect(p.modelName).toBe("juggernautXL_v9Rundiffusionphoto2");
    expect(p.resources).toEqual([
      { type: "checkpoint", modelVersionId: null, modelName: "juggernautXL_v9Rundiffusionphoto2", modelVersionName: null, hash: null, weight: null },
    ]);
  });

  it("handles CRLF line endings and a BOM", () => {
    const p = parseGenerationData("﻿" + PONY_A1111.replace(/\n/g, "\r\n"))!;
    expect(p.prompt.split("\n")).toHaveLength(2);
    expect(p.negative).toBe("score_4, score_5, score_6, lowres, bad anatomy,\nworst quality");
    expect(p.steps).toBe(30);
  });

  it("handles one setting per line", () => {
    const p = parseGenerationData("a cat on a sofa\nSteps: 12\nSampler: Euler a\nCFG scale: 4\nSeed: 9\nSize: 768x512")!;
    expect(p.prompt).toBe("a cat on a sofa");
    expect(p.steps).toBe(12);
    expect(p.sampler).toBe("Euler a");
    expect(p.cfg).toBe(4);
    expect([p.width, p.height]).toEqual([768, 512]);
  });

  it("handles a settings line wrapped by an editor", () => {
    const p = parseGenerationData("a cat\nSteps: 12, Sampler: Euler a, CFG scale: 4,\nSeed: 9, Size: 768x512")!;
    expect(p.prompt).toBe("a cat");
    expect(p.seed).toBe(9);
    expect(p.cfg).toBe(4);
  });

  it("keeps commas of unquoted values that are not keys", () => {
    const p = parseGenerationData("x\nSteps: 10, Model: my model, v2 final, Seed: 5")!;
    expect(p.modelName).toBe("my model, v2 final");
    expect(p.seed).toBe(5);
  });

  it("treats a seed of -1 as random and ignores nonsense numbers", () => {
    const p = parseGenerationData("x\nSteps: banana, Sampler: Euler, Seed: -1, CFG scale: 7, Size: hugexbig")!;
    expect(p.steps).toBeNull();
    expect(p.seed).toBeNull();
    expect(p.width).toBeNull();
    expect(p.cfg).toBe(7);
  });

  it("returns a prompt-only result for plain text", () => {
    const p = parseGenerationData("just a prompt, with commas: and colons")!;
    expect(p.hasSettings).toBe(false);
    expect(p.prompt).toBe("just a prompt, with commas: and colons");
    expect(p.resources).toEqual([]);
  });

  it("does not throw on garbage", () => {
    expect(parseGenerationData("")).toBeNull();
    expect(parseGenerationData("   \n  ")).toBeNull();
    for (const g of [
      "{}",
      "[[[[",
      'Steps: 1, Sampler: "unterminated',
      "Steps: 1, Seed: 2, Civitai resources: [{\"type\":\"lora\",",
      "Negative prompt:",
      "::::,,,,",
      "\u0000\u0001 Steps: 3, Seed: 4",
    ]) {
      expect(() => parseGenerationData(g)).not.toThrow();
    }
    const p = parseGenerationData('x\nSteps: 1, Seed: 2, Civitai resources: [{"type":"lora",')!;
    expect(p.steps).toBe(1);
    expect(p.resources).toEqual([]);
  });

  it("removes lyco/hypernet tags and tidies commas", () => {
    const p = parseGenerationData("a, <lyco:foo:0.5>, b <hypernet:hn:1>, c\nSteps: 2, Seed: 3")!;
    expect(p.prompt).toBe("a, b, c");
    expect(p.loraTags).toEqual([{ name: "foo", weight: 0.5 }]);
    expect(p.hypernetworks).toEqual(["hn"]);
  });

  it("uses a default weight of 1 for tags without a weight", () => {
    const p = parseGenerationData("<lora:thing> a dog\nSteps: 2, Seed: 3")!;
    expect(p.prompt).toBe("a dog");
    expect(p.loraTags).toEqual([{ name: "thing", weight: 1 }]);
  });

  it("merges tag weights and hashes into CivitAI's LoRA list instead of duplicating", () => {
    const text = [
      "a knight <lora:Detailer_v1:0.7>",
      'Steps: 20, Seed: 1, Lora hashes: "Detailer_v1: aaaaaaaaaaaa", Civitai resources: [{"type":"lora","modelVersionId":5,"modelName":"Detailer","modelVersionName":"Detailer_v1"}]',
    ].join("\n");
    const p = parseGenerationData(text)!;
    const loras = p.resources.filter((r) => r.type === "lora");
    expect(loras).toEqual([{ type: "lora", modelVersionId: 5, modelName: "Detailer", modelVersionName: "Detailer_v1", hash: "aaaaaaaaaaaa", weight: 0.7 }]);
  });

  it("puts the Model hash on CivitAI's checkpoint entry", () => {
    const p = parseGenerationData('x\nSteps: 1, Model hash: abcdef1234, Civitai resources: [{"type":"checkpoint","modelVersionId":9}]')!;
    expect(p.resources).toEqual([{ type: "checkpoint", modelVersionId: 9, modelName: null, modelVersionName: null, hash: "abcdef1234", weight: null }]);
  });
});

describe("looksLikeGenerationData", () => {
  it("recognises real samples", () => {
    for (const s of [PONY_A1111, ILLUSTRIOUS_CIVITAI, FLUX_CIVITAI, FLUX_FORGE, A1111_HIRES, COMFY_ISH]) {
      expect(looksLikeGenerationData(s)).toBe(true);
    }
  });
  it("ignores ordinary prompts", () => {
    expect(looksLikeGenerationData("a cat, sitting on a chair, 35mm")).toBe(false);
    expect(looksLikeGenerationData("Style: watercolor, Mood: calm")).toBe(false);
    expect(looksLikeGenerationData("")).toBe(false);
  });
});

describe("tokenizeSettings", () => {
  it("splits quoted, JSON and plain values", () => {
    expect(tokenizeSettings('A: 1, B: "x, y", C: [1, {"d": "e,f"}], D: {"g": [1,2]}, E: plain')).toEqual([
      ["A", "1"],
      ["B", "x, y"],
      ["C", '[1, {"d": "e,f"}]'],
      ["D", '{"g": [1,2]}'],
      ["E", "plain"],
    ]);
  });
  it("unescapes quoted values", () => {
    expect(tokenizeSettings('P: "say \\"hi\\", ok"')).toEqual([["P", 'say "hi", ok']]);
  });
});
