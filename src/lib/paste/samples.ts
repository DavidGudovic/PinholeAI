// Test fixtures: realistic pasted generation data.
// Realistic samples (structure copied from real CivitAI / A1111 / Forge output;
// names and numbers are made up).

export const PONY_A1111 = [
  "score_9, score_8_up, score_7_up, 1girl, solo, <lora:add_detail:0.6>, looking at viewer, cherry blossoms, <lora:Pony_Style_XL:0.8>",
  "upper body, smile",
  "Negative prompt: score_4, score_5, score_6, lowres, bad anatomy,",
  "worst quality",
  'Steps: 30, Sampler: DPM++ 2M Karras, CFG scale: 7, Seed: 3141592653, Size: 832x1216, Model hash: 67ab2fd8ec, Model: ponyDiffusionV6XL_v6StartWithThisOne, VAE hash: 235745af8d, VAE: sdxl_vae.safetensors, Clip skip: 2, Lora hashes: "add_detail: 7c6bad76eb54, Pony_Style_XL: 0123abcd4567", Version: v1.7.0, Hashes: {"vae": "235745af8d", "lora:add_detail": "7c6bad76eb54", "model": "67ab2fd8ec"}',
].join("\n");

export const ILLUSTRIOUS_CIVITAI = [
  "masterpiece, best quality, 1girl, red hair, forest, dappled sunlight",
  "Negative prompt: worst quality, low quality, watermark",
  'Steps: 28, CFG scale: 5.5, Sampler: Euler a, Seed: 987654321, workflow: txt2img, Size: 832x1216, draft: false, Clip skip: 2, Created Date: 2024-11-02T10:11:12.1234567Z, Civitai resources: [{"type":"checkpoint","modelVersionId":1190596,"modelName":"WAI-illustrious-SDXL","modelVersionName":"v11.0"},{"type":"lora","weight":0.8,"modelVersionId":456789,"modelName":"Detailer, Illustrious","modelVersionName":"v1.0"},{"type":"embed","weight":1,"modelVersionId":222,"modelName":"lazyneg","modelVersionName":"v1"}], Civitai metadata: {"remixOfId":12345}',
].join("\n");

export const FLUX_CIVITAI = [
  "A cinematic photo of an old fisherman mending nets at dawn, warm light, mist over the harbor",
  'Steps: 20, CFG scale: 3.5, Sampler: Undefined, Seed: 1234567890, Size: 832x1216, Clip skip: 1, Created Date: 2024-09-01T00:00:00.000Z, Civitai resources: [{"type":"checkpoint","modelVersionId":691639,"modelName":"FLUX","modelVersionName":"Dev"}], Civitai metadata: {}',
].join("\n");

export const FLUX_FORGE = [
  "portrait of a woman in a yellow raincoat, city street at night, neon reflections",
  "Steps: 25, Sampler: Euler, Schedule type: Simple, CFG scale: 1, Distilled CFG Scale: 3.5, Seed: 42, Size: 896x1152, Model hash: d5d1e2a8b0, Model: flux1-dev-Q8_0, Version: f2.0.1v1.10.1-previous-313-g8a042934, Module 1: ae, Module 2: clip_l, Module 3: t5xxl_fp16",
].join("\n");

export const A1111_HIRES = [
  "photo of a cozy cabin in snowy woods, golden hour",
  "Negative prompt: (worst quality:1.4), cartoon",
  'Steps: 25, Sampler: DPM++ 2M, Schedule type: Karras, CFG scale: 6, Seed: 777, Size: 512x768, Model hash: 6ce0161689, Model: v1-5-pruned-emaonly, Denoising strength: 0.45, Clip skip: 1, ADetailer model: face_yolov8n.pt, ADetailer confidence: 0.3, ADetailer prompt: "detailed face, smile", ADetailer version: 24.1.2, Hires upscale: 2, Hires steps: 10, Hires upscaler: R-ESRGAN 4x+, Version: v1.9.4',
].join("\n");

export const COMFY_ISH = [
  "a red fox in the snow, highly detailed",
  "Negative prompt: blurry",
  "Steps: 30, Sampler: dpmpp_2m_sde_gpu, Scheduler: karras, CFG scale: 6.5, Seed: 123, Size: 1024x1024, Model: juggernautXL_v9Rundiffusionphoto2, Denoise: 1",
].join("\n");
