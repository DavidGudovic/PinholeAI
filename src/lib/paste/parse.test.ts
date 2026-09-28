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
      { type: "checkpoint", modelVersionId: 1190596, modelName: "WAI-illustrious-SDXL", modelVersionName: "v11.0", hash: null, weight: null },
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

  it("reads CivitAI AIRs when there is no modelVersionId", () => {
    const p = parseGenerationData(
      'x\nSteps: 1, Civitai resources: [{"air":"urn:air:sdxl:checkpoint:civitai:827184@1190596"},{"type":"lora","weight":0.8,"air":"urn:air:sdxl:lora:civitai:12@34"},{"air":"urn:air:sdxl:checkpoint:huggingface:a/b@c"}]',
    )!;
    expect(p.resources).toEqual([
      { type: "checkpoint", modelVersionId: 1190596, modelName: null, modelVersionName: null, hash: null, weight: null },
      { type: "lora", modelVersionId: 34, modelName: null, modelVersionName: null, hash: null, weight: 0.8 },
    ]);
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

describe("LoRA de-duplication across Civitai resources / Lora hashes / Hashes", () => {
  const loras = (text: string) => parseGenerationData(text)!.resources.filter((r) => r.type === "lora");

  it("lists a LoRA once when CivitAI and Lora hashes name it differently (real CivitAI copy)", () => {
    const text = [
      "portrait of a knight, dramatic lighting",
      "Negative prompt: lowres",
      'Steps: 28, Sampler: DPM++ 2M Karras, CFG scale: 6.5, Seed: 1234567, Size: 512x768, Model hash: 6ce0161689, Model: v1-5-pruned-emaonly, Lora hashes: "detail_tweaker: e3f9c2b1a8d7", Civitai resources: [{"type":"checkpoint","modelVersionId":128713,"modelName":"DreamShaper","modelVersionName":"8"},{"type":"lora","weight":0.6,"modelVersionId":62833,"modelName":"Detail Tweaker LoRA","modelVersionName":"v1.0"}], Version: v1.9.4',
    ].join("\n");
    expect(loras(text)).toEqual([
      { type: "lora", modelVersionId: 62833, modelName: "Detail Tweaker LoRA", modelVersionName: "v1.0", hash: "e3f9c2b1a8d7", weight: 0.6 },
    ]);
  });

  it("pairs the one leftover CivitAI LoRA with the one leftover hashed LoRA", () => {
    const text =
      'x <lora:ftw_v3:0.7>\nSteps: 20, Seed: 1, Lora hashes: "ftw_v3: 0123456789ab", Civitai resources: [{"type":"lora","modelVersionId":77,"modelName":"Film Look"}]';
    expect(loras(text)).toEqual([{ type: "lora", modelVersionId: 77, modelName: "Film Look", modelVersionName: null, hash: "0123456789ab", weight: 0.7 }]);
  });

  it("keeps LoRAs apart when they really differ", () => {
    const text =
      'x\nSteps: 20, Seed: 1, Lora hashes: "alpha_style: 111111111111, beta: 222222222222", Civitai resources: [{"type":"lora","modelVersionId":1,"modelName":"Alpha Style"}]';
    const got = loras(text);
    expect(got).toHaveLength(2);
    expect(got[0]).toMatchObject({ modelVersionId: 1, modelName: "Alpha Style", hash: "111111111111" });
    expect(got[1]).toMatchObject({ modelVersionId: null, modelName: "beta", hash: "222222222222" });
    // Hashes that disagree are never paired.
    const clash =
      'x\nSteps: 20, Seed: 1, Lora hashes: "zzz: 333333333333", Civitai resources: [{"type":"lora","modelVersionId":9,"modelName":"Other","hash":"444444444444"}]';
    expect(loras(clash)).toHaveLength(2);
  });

  it("merges the same file named twice (Hashes JSON + Lora hashes, CivitAI hash)", () => {
    const text =
      'x\nSteps: 20, Seed: 1, Lora hashes: "Add_Detail: aaaaaaaaaaaa", Hashes: {"lora:add_detail": "aaaaaaaaaaaa", "model": "bbbbbbbbbb"}';
    expect(loras(text)).toEqual([{ type: "lora", modelVersionId: null, modelName: "Add_Detail", modelVersionName: null, hash: "aaaaaaaaaaaa", weight: null }]);
    const withId =
      'x\nSteps: 20, Seed: 1, Lora hashes: "some_file: cccccccccccc", Civitai resources: [{"type":"lora","modelVersionId":3,"modelName":"Unrelated Name","hash":"cccccccccccc","weight":0.5}]';
    expect(loras(withId)).toEqual([{ type: "lora", modelVersionId: 3, modelName: "Unrelated Name", modelVersionName: null, hash: "cccccccccccc", weight: 0.5 }]);
  });
});
