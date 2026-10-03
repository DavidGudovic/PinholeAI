// Mock handlers for the generate area: family UI, generation with progress
// events, the in-memory image session, save/copy/upscale, prompt preview.
// Fake images are painted on a canvas from the seed — never from prompt text.
import { invoke } from "@tauri-apps/api/core";
import type { MockTable } from "./index";
import { mockEmit } from "./index";
import { styleById } from "./library";
import { mockFlags, mockSettings } from "./app";
import { touchLastUsed } from "./models";
import { FAMILY_UI } from "../state/familyFixtures";
import type {
  CoreError,
  EngineStatus,
  FamilyUi,
  FinalPromptPreview,
  GenerateRequest,
  GenerationProgress,
  ImportedImage,
  InstalledLora,
  InstalledModel,
  ImageOrigin,
  ResultImage,
  SafetyCheckStatus,
} from "../types";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const err = (code: string, message: string, details: string | null = null): CoreError => ({ code, message, details });

// ---------------------------------------------------------------- out of graphics memory (?busygpu)
// Mirrors Rust generate.rs: with 9 GB held by another program, reading the prompt runs out of
// graphics memory once per model; Settings textEncoderOnCpu "auto" retries with the text encoder
// on the processor (remembered per model until the app closes), "off" fails with code "vram".
const BUSY_NOTE = "Other programs are using 9 GB of your graphics memory: python.exe (8.9 GB). If pictures fail, close them and try again.";
const TE_RETRY_NOTE = "Your graphics card ran out of memory while reading your prompt — trying again with that step on the processor (a bit slower).";
const TE_ON_GPU_MESSAGE =
  "Your graphics card ran out of memory while reading your prompt. In Settings → Engine, set “Read the prompt on the processor” to Automatic or On, or close other programs that use the graphics card and try again.";
const TE_OOM_DETAILS = [
  "generate_image returned no results",
  "[WARN   ] model_manager.cpp:1914 - model manager cannot make enough memory available on CUDA0: need 518.58 MB device / 6.58 MB budget, available 0.00 MB device / 7044.91 MB budget",
  "[ERROR  ] ggml_runner.cpp:898  - qwen3 segment 1/1 (graph) failed during workspace capacity check",
  "[ERROR  ] conditioner.hpp:2224 - LLM prompt encoding failed",
  "[ERROR  ] image.cpp:448  - failed to encode prompt",
].join("\n");
/** Models whose text encoder moved to the processor after running out of memory (app session). */
const teOnCpu = new Set<string>();

// ---------------------------------------------------------------- session (RAM)
interface SessionImage {
  bytes: Uint8Array;
  width: number;
  height: number;
  seed: number;
  /** Generated/upscaled images only (imported ones have none), like Rust. */
  meta?: ResultImage;
}
const session = new Map<string, SessionImage>();
let idSeq = 0;
const newId = () => `img-${(++idSeq).toString(36)}-${Math.random().toString(36).slice(2, 7)}`;

function mustGet(id: unknown): SessionImage {
  const im = session.get(String(id));
  if (!im) throw err("not_found", "That image is no longer in memory (it was cleared). Generate it again.");
  return im;
}

// ---------------------------------------------------------------- family UI
function familyUi(familyId: string): FamilyUi {
  const known = FAMILY_UI[familyId];
  if (known) return known;
  const isEdit = /edit|kontext/.test(familyId);
  return { ...FAMILY_UI.sdxl, familyId, label: familyId.replace(/_/g, " "), isEditFamily: isEdit, modes: isEdit ? ["edit"] : ["txt2img", "img2img"] };
}

// ---------------------------------------------------------------- painting
function rand(seed: number) {
  let s = seed >>> 0 || 1;
  return () => {
    s ^= s << 13;
    s ^= s >>> 17;
    s ^= s << 5;
    return ((s >>> 0) % 100000) / 100000;
  };
}

function canvas(w: number, h: number) {
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  return c;
}

async function toPng(c: HTMLCanvasElement): Promise<Uint8Array> {
  const blob = await new Promise<Blob | null>((res) => c.toBlob(res, "image/png"));
  if (!blob) throw err("internal", "Couldn't encode the image.");
  return new Uint8Array(await blob.arrayBuffer());
}

/** A calm landscape-ish abstract from the seed (sky, sun, layered hills, grain). */
function paintScene(w: number, h: number, seed: number, familyId: string): HTMLCanvasElement {
  const c = canvas(w, h);
  const g = c.getContext("2d")!;
  const r = rand(seed * 2654435761);
  const anime = /illustrious|pony/.test(familyId);
  const hue = Math.floor(r() * 360);
  const sky = g.createLinearGradient(0, 0, 0, h);
  sky.addColorStop(0, `hsl(${hue}, ${anime ? 70 : 45}%, ${anime ? 72 : 62}%)`);
  sky.addColorStop(0.55, `hsl(${(hue + 30) % 360}, ${anime ? 80 : 55}%, ${anime ? 80 : 70}%)`);
  sky.addColorStop(1, `hsl(${(hue + 60) % 360}, 50%, 45%)`);
  g.fillStyle = sky;
  g.fillRect(0, 0, w, h);

  // sun
  const sx = w * (0.2 + r() * 0.6);
  const sy = h * (0.18 + r() * 0.22);
  const sr = Math.min(w, h) * (0.07 + r() * 0.06);
  const glow = g.createRadialGradient(sx, sy, sr * 0.2, sx, sy, sr * 4);
  glow.addColorStop(0, "rgba(255, 244, 214, 0.95)");
  glow.addColorStop(0.25, "rgba(255, 209, 102, 0.55)");
  glow.addColorStop(1, "rgba(255, 209, 102, 0)");
  g.fillStyle = glow;
  g.fillRect(0, 0, w, h);
  g.fillStyle = "rgba(255, 250, 235, 0.95)";
  g.beginPath();
  g.arc(sx, sy, sr, 0, Math.PI * 2);
  g.fill();

  // hills
  const layers = 4;
  for (let i = 0; i < layers; i++) {
    const base = h * (0.52 + i * 0.12);
    const amp = h * (0.08 - i * 0.012) * (0.6 + r());
    const freq = 1.5 + r() * 2.5;
    const phase = r() * Math.PI * 2;
    g.beginPath();
    g.moveTo(0, h);
    for (let x = 0; x <= w; x += Math.max(4, w / 160)) {
      const y = base + Math.sin((x / w) * Math.PI * freq + phase) * amp + Math.sin((x / w) * Math.PI * freq * 3.1 + phase * 2) * amp * 0.25;
      g.lineTo(x, y);
    }
    g.lineTo(w, h);
    g.closePath();
    g.fillStyle = `hsl(${(hue + 200 + i * 12) % 360}, ${30 + i * 6}%, ${38 - i * 8}%)`;
    g.fill();
  }

  // grain
  const img = g.getImageData(0, 0, w, h);
  const d = img.data;
  const rg = rand(seed + 7);
  for (let i = 0; i < d.length; i += 4 * 3) {
    const n = (rg() - 0.5) * 14;
    d[i] += n;
    d[i + 1] += n;
    d[i + 2] += n;
  }
  g.putImageData(img, 0, 0);

  // small caption with settings only (never prompt text)
  g.font = `${Math.max(11, Math.round(w / 64))}px system-ui, sans-serif`;
  g.fillStyle = "rgba(255,255,255,0.7)";
  g.fillText(`Pinhole mock · seed ${seed} · ${w}×${h}`, Math.round(w / 48), h - Math.round(w / 48));
  return c;
}

async function decode(bytes: Uint8Array): Promise<ImageBitmap> {
  return createImageBitmap(new Blob([bytes as BlobPart]));
}

/** Instruction edit / restyle: recolour the source (inside the mask when there is one). */
async function paintEdit(src: SessionImage, w: number, h: number, seed: number, strength: number, mask: SessionImage | null): Promise<HTMLCanvasElement> {
  const bmp = await decode(src.bytes);
  const out = canvas(w, h);
  const g = out.getContext("2d")!;
  g.drawImage(bmp, 0, 0, w, h);
  const fx = canvas(w, h);
  const f = fx.getContext("2d")!;
  const r = rand(seed);
  f.filter = `hue-rotate(${Math.round(40 + r() * 200 * strength)}deg) saturate(${1 + strength}) contrast(${1 + strength * 0.2})`;
  f.drawImage(bmp, 0, 0, w, h);
  f.filter = "none";
  if (mask) {
    // White in the mask = change. Turn luminance into alpha and keep the effect only there.
    const mb = await decode(mask.bytes);
    const m = canvas(w, h);
    const mg = m.getContext("2d")!;
    mg.drawImage(mb, 0, 0, w, h);
    const md = mg.getImageData(0, 0, w, h);
    for (let i = 0; i < md.data.length; i += 4) {
      md.data[i + 3] = md.data[i];
      md.data[i] = md.data[i + 1] = md.data[i + 2] = 255;
    }
    mg.putImageData(md, 0, 0);
    f.globalCompositeOperation = "destination-in";
    f.drawImage(m, 0, 0);
    f.globalCompositeOperation = "source-over";
  }
  g.drawImage(fx, 0, 0);
  return out;
}

// ---------------------------------------------------------------- generation
let cancelled = false;
let loadedModel: string | null = null;
let running = false;

const progress = (p: Partial<GenerationProgress> & { phase: GenerationProgress["phase"] }, started: number) =>
  mockEmit("generation-progress", { modelLabel: null, queuePosition: null, step: null, totalSteps: null, elapsedMs: Date.now() - started, ...p });

async function checkCancel(started: number) {
  if (cancelled) {
    progress({ phase: "cancelled" }, started);
    throw err("cancelled", "Stopped.");
  }
}

async function generate(req: GenerateRequest): Promise<{ images: ResultImage[] }> {
  if (running) throw err("invalid", "Pinhole is still working on the last image.");
  // Stand-in for the Rust word check (text_check.rs), for trying the block screen in the mock.
  if (/\bminor\b/i.test(req.prompt) && /\bexplicit\b/i.test(req.prompt))
    throw err("blocked", "Pinhole can't help with this. See the usage guidelines.");
  running = true;
  cancelled = false;
  const started = Date.now();
  try {
    // Like Rust: the check's files first (ensure_ready), then the engine.
    await requireCheck();
    const engine = await invoke<EngineStatus>("engine_status").catch(() => null);
    if (engine && !engine.installed) throw err("engine_missing", "The image engine isn't set up yet. It's a one-time download — click “Get the engine”.");
    const models = await invoke<InstalledModel[]>("list_models");
    const model = models.find((m) => m.id === req.modelId);
    if (!model) throw err("not_found", "That model isn't installed any more. Pick another one.");
    if (model.missingComponents.length)
      throw err("not_found", `This model still needs ${model.missingComponents.join(", ")}. Open Models → Installed to finish setting it up.`);
    const familyId = model.familyId ?? "sdxl";
    const ui = familyUi(familyId);
    const q = req.dials.quality === "fast" ? 0 : req.dials.quality === "best" ? 2 : 1;
    const steps = req.fineTune.steps ?? ui.qualitySteps[q];
    const [sw, sh] = ui.shapes[req.dials.shape] ?? [1024, 1024];
    let w = req.fineTune.width ?? sw;
    let h = req.fineTune.height ?? sh;
    const stickVal = ui.stickRange[0] + Math.min(1, Math.max(0, req.dials.stick)) * (ui.stickRange[1] - ui.stickRange[0]);
    const cfg = Math.max(1, req.fineTune.cfg ?? (ui.stickMapsTo === "cfg" && ui.showStick ? stickVal : ui.defaultCfg));
    const guidance = req.fineTune.guidance ?? (ui.stickMapsTo === "guidance" ? stickVal : ui.defaultGuidance);
    const count = req.mode === "txt2img" ? req.dials.count : 1;
    const baseSeed = req.fineTune.seed ?? Math.floor(Math.random() * 2 ** 31);

    const src = req.mode === "edit" ? session.get(req.refImageIds?.[0] ?? "") : req.mode === "img2img" ? session.get(req.initImageId ?? "") : undefined;
    if (req.mode !== "txt2img" && !src) throw err("not_found", "The image to edit is no longer in memory. Add it again.");
    // Create's reference picture (mirrors generate.rs): only generators that can also edit take one.
    if (req.mode === "txt2img" && req.refImageIds?.length) {
      if (!model.modes.includes("edit")) throw err("invalid", "This model can't use a reference picture. Pick a FLUX.2 model, or remove the picture.");
      if (!session.get(req.refImageIds[0])) throw err("not_found", "The reference picture is no longer in memory. Add it again.");
    }
    const mask = req.maskImageId ? (session.get(req.maskImageId) ?? null) : null;
    // Like Rust: a result is "imported" if any picture it's made from is.
    const inputs = [...(req.refImageIds ?? []).slice(0, req.mode === "edit" ? 2 : 1), ...(req.mode === "img2img" && req.initImageId ? [req.initImageId] : [])];
    const originOf = (id: string) => session.get(id)?.meta?.origin ?? "imported";
    const origin: ImageOrigin = inputs.some((id) => originOf(id) !== "generated") ? "imported" : "generated";

    // Model switch → "Loading <model>… (~10–30 s)" (shortened here).
    // Rust measures other programs' graphics memory (nvidia-smi) before a launch.
    const busy = mockFlags().busyGpu;
    const load = async (note: string | null) => {
      const tensors = 1130;
      for (let i = 0; i <= 8; i++) {
        progress({ phase: "loadingModel", modelLabel: model.friendlyName, step: Math.round((tensors * i) / 8), totalSteps: tensors, note }, started);
        await sleep(200);
        await checkCancel(started);
      }
      loadedModel = model.id;
    };
    if (loadedModel !== model.id) await load(busy ? BUSY_NOTE : null);
    const teSetting = mockSettings().textEncoderOnCpu;
    if (busy && teSetting !== "on" && !(teSetting === "auto" && teOnCpu.has(model.id))) {
      progress({ phase: "queued", queuePosition: 1 }, started);
      await sleep(400);
      if (teSetting === "off") throw err("vram", TE_ON_GPU_MESSAGE, TE_OOM_DETAILS);
      teOnCpu.add(model.id);
      await load(`${BUSY_NOTE} ${TE_RETRY_NOTE}`);
    }
    progress({ phase: "queued", queuePosition: 1 }, started);
    await sleep(250);

    const images: ResultImage[] = [];
    for (let n = 0; n < count; n++) {
      for (let s = 1; s <= steps; s++) {
        await checkCancel(started);
        progress({ phase: "generating", step: s + n * steps, totalSteps: steps * count }, started);
        await sleep(Math.max(25, Math.min(110, 1400 / steps)));
      }
      const seed = baseSeed + n;
      let c: HTMLCanvasElement;
      if (src && req.extend) {
        // Extend (mirrors Rust extend.rs, roughly): a scene on the bigger canvas, the source pasted back.
        ({ width: w, height: h } = req.extend);
        c = paintScene(w, h, seed, familyId);
        c.getContext("2d")!.drawImage(await decode(src.bytes), req.extend.left, req.extend.top, src.width, src.height);
      } else if (src) {
        if (!req.fineTune.width) [w, h] = [src.width, src.height];
        c = await paintEdit(src, w, h, seed, req.mode === "edit" ? 0.6 : (req.strength ?? 0.55), mask);
      } else {
        c = paintScene(w, h, seed, familyId);
      }
      const id = newId();
      const meta: ResultImage = {
        id,
        kind: "generated",
        width: w,
        height: h,
        seed,
        modelId: model.id,
        modelLabel: model.friendlyName,
        familyId,
        steps,
        cfg: Math.round(cfg * 100) / 100,
        guidance: guidance != null ? Math.round(guidance * 100) / 100 : null,
        sampler: req.fineTune.sampler ?? ui.defaultSampler,
        scheduler: req.fineTune.scheduler ?? ui.defaultScheduler,
        parentId: src ? (req.refImageIds?.[0] ?? req.initImageId ?? null) : null,
        origin,
        ...(ui.seamless && req.mode === "txt2img" && req.fineTune.seamless ? { seamless: true } : {}),
      };
      session.set(id, { bytes: await toPng(c), width: w, height: h, seed, meta });
      images.push(meta);
    }
    touchLastUsed(model.id);
    progress({ phase: "done" }, started);
    return { images };
  } finally {
    running = false;
  }
}

// ---------------------------------------------------------------- prompt preview (mirrors registry style::combine)
async function preview(req: GenerateRequest): Promise<FinalPromptPreview> {
  const models = await invoke<InstalledModel[]>("list_models");
  const model = models.find((m) => m.id === req.modelId);
  const ui = familyUi(model?.familyId ?? "sdxl");
  const natural = /flux|z_image|qwen/.test(ui.familyId);
  const style = styleById(req.styleId);
  let prompt = req.prompt.trim();
  // Add detail with nothing typed: the mock treats every picture as a photo.
  if (!prompt && req.fixDetails && req.mode === "img2img" && !req.maskImageId && req.initImageId)
    prompt = req.styleId ? "a detailed face" : "photo of a face, natural skin texture, sharp focus";
  if (req.addTriggerWords && req.loras.length) {
    const loras = await invoke<InstalledLora[]>("list_loras");
    const words: string[] = [];
    for (const u of req.loras) {
      const all = loras.find((l) => l.id === u.loraId)?.trainedWords ?? [];
      for (const w of u.words ? all.filter((w) => u.words!.some((p) => p.trim().toLowerCase() === w.toLowerCase())) : all)
        if (!words.some((x) => x.toLowerCase() === w.toLowerCase())) words.push(w);
    }
    const missing = words.filter((w) => !new RegExp(`(^|[^\\p{L}\\p{N}])${w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}($|[^\\p{L}\\p{N}])`, "iu").test(prompt));
    if (missing.length) prompt = prompt ? `${prompt}, ${missing.join(", ")}` : missing.join(", ");
  }
  if (style) prompt = natural ? `${prompt}. Style: ${style.positive}` : `${prompt}, ${style.positive}`;
  if (ui.autoPromptPrefix && req.fineTune.autoPromptPrefix !== false) prompt = `${ui.autoPromptPrefix}${prompt}`;
  let negative: string | null = null;
  if (ui.usesNegativePrompt) {
    const parts = [req.fineTune.negativePrompt?.trim() || ui.defaultNegativePrompt, style?.negative].filter(Boolean);
    negative = parts.length ? parts.join(", ") : null;
  }
  return { prompt, negative };
}

// ---------------------------------------------------------------- table
const stamp = () => {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}_${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`;
};

/** Like Rust `imagecheck::ensure_ready`: nothing is made without the check's files. */
async function requireCheck() {
  const check = await invoke<SafetyCheckStatus>("safety_check_status").catch(() => null);
  if (check && !check.ready)
    throw err(
      "check_missing",
      check.downloading
        ? "Pinhole's safety check is still downloading (see Downloads). Try again when it's done."
        : "Pinhole's safety check isn't set up yet. Click “Set up safety check” to download it (about 1.2 GB), then try again.",
    );
}

const table: MockTable = {
  family_ui: async (a) => {
    await sleep(40);
    return familyUi(String(a.familyId));
  },
  generate: (a) => generate(a.req as GenerateRequest),
  cancel_generation: async () => {
    if (running) cancelled = true;
  },
  preview_final_prompt: (a) => preview(a.req as GenerateRequest),
  import_image: async (a) => {
    // Raw binary body: `a` is the Uint8Array itself.
    const bytes = a instanceof Uint8Array ? a : new Uint8Array(a as unknown as ArrayBuffer);
    let bmp: ImageBitmap;
    try {
      bmp = await decode(bytes);
    } catch {
      throw err("invalid", "That file isn't an image Pinhole can open. Try a PNG, JPEG or WebP.");
    }
    const id = newId();
    session.set(id, { bytes, width: bmp.width, height: bmp.height, seed: 0 });
    return { id, width: bmp.width, height: bmp.height } satisfies ImportedImage;
  },
  read_picture_settings: async (a) => {
    // Raw body. Reads the PNG "pinhole" text chunk like the Rust side (no range checks).
    const b = a instanceof Uint8Array ? a : new Uint8Array(a as unknown as ArrayBuffer);
    const dv = new DataView(b.buffer, b.byteOffset, b.byteLength);
    for (let pos = 8; pos + 12 <= b.length; ) {
      const len = dv.getUint32(pos);
      const kind = String.fromCharCode(...b.slice(pos + 4, pos + 8));
      if (kind === "tEXt") {
        const text = new TextDecoder("latin1").decode(b.slice(pos + 8, pos + 8 + len));
        if (text.startsWith("pinhole\0")) {
          try {
            const v = JSON.parse(text.slice(8));
            return v?.app === "Pinhole" ? v : null;
          } catch {
            return null;
          }
        }
      }
      pos += 12 + len;
    }
    return null;
  },
  get_image: async (a) => {
    const im = mustGet(a.id);
    return im.bytes.slice().buffer;
  },
  save_image: async (a) => {
    const im = mustGet(a.id);
    await sleep(150);
    return { path: `~/.local/share/pinhole/Data/outputs/pinhole_${stamp()}_${im.seed}.png` };
  },
  save_image_as: async (a) => {
    mustGet(a.id);
    await sleep(150);
    return { path: String(a.path) };
  },
  save_images_to: async (a) => {
    const ids = a.ids as string[];
    await sleep(150);
    return { saved: ids.map((id) => ({ id, path: `${String(a.dir)}/pinhole_${stamp()}_${mustGet(id).seed}.png` })), failed: 0 };
  },
  copy_image: async (a) => {
    const im = mustGet(a.id);
    try {
      await navigator.clipboard.write([new ClipboardItem({ "image/png": new Blob([im.bytes as BlobPart], { type: "image/png" }) })]);
    } catch {
      /* browser may refuse without a user gesture; the real app copies from Rust */
    }
  },
  clipboard_image: async () => new ArrayBuffer(0),
  discard_image: async (a) => {
    session.delete(String(a.id));
  },
  clear_session: async () => {
    session.clear();
  },
  upscale_image: async (a) => {
    const im = mustGet(a.id);
    const factor = Number(a.factor) === 4 ? 4 : 2;
    // Like Rust: upscales are checked like every made picture.
    await requireCheck();
    if (running) throw err("invalid", "Pinhole is still working on the last image.");
    running = true;
    cancelled = false;
    const started = Date.now();
    try {
      for (let s = 1; s <= 10; s++) {
        await checkCancel(started);
        progress({ phase: "generating", step: s, totalSteps: 10 }, started);
        await sleep(120);
      }
      // Cap the mock's canvas so 4× stays light in the browser.
      const cap = 3072 / Math.max(im.width * factor, im.height * factor);
      const w = Math.round(im.width * factor * Math.min(1, cap));
      const h = Math.round(im.height * factor * Math.min(1, cap));
      const bmp = await decode(im.bytes);
      const c = canvas(w, h);
      const g = c.getContext("2d")!;
      g.imageSmoothingQuality = "high";
      g.drawImage(bmp, 0, 0, w, h);
      const id = newId();
      // Like Rust: copy the source image's settings; an imported source has none.
      const base: ResultImage = im.meta ?? {
        id: "",
        width: 0,
        height: 0,
        seed: 0,
        modelId: "",
        modelLabel: "Upscaled image",
        familyId: "",
        steps: 0,
        cfg: 0,
        guidance: null,
        sampler: null,
        scheduler: null,
        parentId: null,
      };
      // Rust picks by the picture's photo-style reading; the mock has none, so "auto" means photo.
      const choice = mockSettings().upscaler ?? "auto";
      const upscaler = choice === "auto" ? "photo" : choice;
      const meta: ResultImage = { ...base, id, kind: "upscaled", width: w, height: h, parentId: String(a.id), origin: im.meta?.origin ?? "imported", upscaler };
      session.set(id, { bytes: await toPng(c), width: w, height: h, seed: base.seed, meta });
      progress({ phase: "done" }, started);
      return meta;
    } finally {
      running = false;
    }
  },
};

export default table;
