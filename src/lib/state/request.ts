// Pure builders: app state → GenerateRequest, presets ↔ Create params.
// PRIVACY: requests contain prompt text — memory only. Presets never do.

import { defaultStickPosition, shapeFor } from "../paste/map";
import type {
  Dials,
  FamilyUi,
  FineTune,
  GenerateRequest,
  InstalledLora,
  InstalledModel,
  LoraUse,
  Preset,
  PresetFineTune,
  PresetLora,
  ResultImage,
  Settings,
} from "../types";
import { CHANGE_STRENGTH, compactFineTune, loraCompatible, type CreateParams, type EditMode, type EditParams, type ImgRef, type PresetBase } from "./model";

/** Fine-tune values that may be stored in a preset (never the negative prompt). */
export const PRESET_FINE_TUNE_KEYS = [
  "sampler",
  "scheduler",
  "steps",
  "cfg",
  "guidance",
  "seed",
  "flowShift",
  "clipSkip",
  "width",
  "height",
  "hires",
  "vaeTiling",
  "autoPromptPrefix",
] as const satisfies readonly (keyof PresetFineTune)[];

export function createDials(c: CreateParams, ui: FamilyUi | null): Dials {
  return { shape: c.shape, quality: c.quality, stick: c.stick ?? defaultStickPosition(ui), count: c.count };
}

// Rust types: steps/width/height u32, clipSkip i32, seed i64. A value serde can't
// parse (typed "-5", "2.5") would fail the whole IPC call with a raw serde error.
const UINT_KEYS = ["steps", "width", "height"] as const;
const INT_KEYS = ["clipSkip", "seed"] as const;

/** Drop numbers the Rust side can't deserialize (= use the registry default). */
function dropInvalidNumbers<T extends PresetFineTune>(ft: T): T {
  const out = { ...ft };
  for (const k of UINT_KEYS) {
    const v = out[k];
    if (v != null && !(Number.isInteger(v) && v >= 0)) delete out[k];
  }
  for (const k of INT_KEYS) {
    const v = out[k];
    if (v != null && !Number.isSafeInteger(v)) delete out[k];
  }
  return out;
}

/** Drop overrides the family can't use. */
export function effectiveFineTune(ft: FineTune, ui: FamilyUi | null): FineTune {
  const out = dropInvalidNumbers(compactFineTune(ft));
  if (ui && !ui.usesNegativePrompt) delete out.negativePrompt;
  if (out.negativePrompt != null && !out.negativePrompt.trim()) delete out.negativePrompt;
  if (ui && !ui.autoPromptPrefix) delete out.autoPromptPrefix;
  if (!out.hires) {
    delete out.hiresScale;
    delete out.hiresDenoise;
  }
  return out;
}

export function activeLoras(c: CreateParams, loras: InstalledLora[], model: InstalledModel | null): LoraUse[] {
  return c.loras.filter((u) => {
    const l = loras.find((x) => x.id === u.loraId);
    return !!l && loraCompatible(l, model?.familyId);
  });
}

export function buildCreateRequest(
  c: CreateParams,
  opts: { ui: FamilyUi | null; loras: InstalledLora[]; model: InstalledModel; settings: Settings | null },
): GenerateRequest {
  return {
    modelId: opts.model.id,
    mode: "txt2img",
    prompt: c.prompt.trim(),
    styleId: c.styleId,
    dials: createDials(c, opts.ui),
    fineTune: effectiveFineTune(c.fineTune, opts.ui),
    loras: activeLoras(c, opts.loras, opts.model),
    addTriggerWords: c.addTriggerWords ?? opts.settings?.addTriggerWords ?? true,
  };
}

/** Same request with a new random seed ("Variations"). */
export function variationRequest(req: GenerateRequest): GenerateRequest {
  const fineTune = { ...req.fineTune };
  delete fineTune.seed;
  return { ...req, fineTune };
}

/** Output size for an edit: keep the aspect ratio, ≤ ~maxPixels, multiples of `multiple` (16, or 64 for SD families). */
export function fitEditSize(w: number, h: number, maxPixels = 1024 * 1024, multiple = 16): [number, number] {
  if (!(w > 0 && h > 0)) return [1024, 1024];
  const scale = Math.min(1, Math.sqrt(maxPixels / (w * h)));
  const round = (n: number) => Math.max(256, Math.round((n * scale) / multiple) * multiple);
  return [round(w), round(h)];
}

/** The Edit tab's "Output size" choice, relative to the source image. */
export type EditSizeChoice = "smaller" | "normal" | "larger";

const EDIT_SIZE_SCALE: Record<EditSizeChoice, number> = { smaller: 0.75, normal: 1, larger: 1.25 };
const EDIT_MAX_PIXELS = 1280 * 1280;

/**
 * Output size for an edit from the Smaller / Normal / Larger choice. "Normal" keeps the source size
 * (shrunk to ≤ ~1 MP); Smaller and Larger are 0.75× / 1.25× of that side length, so each choice
 * changes the result even for small sources (Larger is capped at ~1.6 MP).
 */
export function editOutputSize(w: number, h: number, choice: EditSizeChoice, multiple = 16): [number, number] {
  if (!(w > 0 && h > 0)) return [1024, 1024];
  const base = Math.min(1, Math.sqrt((1024 * 1024) / (w * h)));
  const wanted = Math.min(base * EDIT_SIZE_SCALE[choice], Math.sqrt(EDIT_MAX_PIXELS / (w * h)));
  const round = (n: number) => Math.max(256, Math.round((n * wanted) / multiple) * multiple);
  return [round(w), round(h)];
}

export function buildEditRequest(
  e: EditParams,
  opts: {
    mode: EditMode;
    source: ImgRef;
    model: InstalledModel;
    ui: FamilyUi | null;
    maskImageId: string | null;
    size: [number, number];
  },
): GenerateRequest {
  const fineTune: FineTune = { width: opts.size[0], height: opts.size[1] };
  if (e.seed != null) fineTune.seed = e.seed;
  const shape = shapeFor(opts.ui, opts.size[0], opts.size[1]).shape;
  if (opts.mode === "instruction") {
    return {
      modelId: opts.model.id,
      mode: "edit",
      prompt: e.instruction.trim(),
      styleId: e.styleId,
      dials: { shape, quality: e.quality, stick: e.stayClose ?? defaultStickPosition(opts.ui), count: 1 },
      fineTune,
      loras: [],
      addTriggerWords: false,
      refImageIds: [opts.source.id],
      maskImageId: opts.maskImageId,
    };
  }
  return {
    modelId: opts.model.id,
    mode: "img2img",
    prompt: e.restylePrompt.trim(),
    styleId: e.styleId,
    dials: { shape, quality: e.quality, stick: defaultStickPosition(opts.ui), count: 1 },
    fineTune,
    loras: [],
    addTriggerWords: false,
    initImageId: opts.source.id,
    strength: CHANGE_STRENGTH[e.change],
    maskImageId: opts.maskImageId,
  };
}

// ------------------------------------------------------------------ presets

/** Build a preset from the Create tab. NEVER includes the prompt or negative prompt. */
export function presetFromCreate(
  name: string,
  c: CreateParams,
  opts: { model: InstalledModel | null; loras: InstalledLora[]; id?: string },
): Preset {
  let fineTune: PresetFineTune = {};
  for (const k of PRESET_FINE_TUNE_KEYS) {
    const v = c.fineTune[k];
    if (v !== undefined && v !== null) (fineTune as Record<string, unknown>)[k] = v;
  }
  fineTune = dropInvalidNumbers(fineTune);
  const loras: PresetLora[] = c.loras.map((u) => {
    const l = opts.loras.find((x) => x.id === u.loraId);
    return { loraId: u.loraId, civitaiVersionId: l?.civitaiVersionId ?? null, name: l?.friendlyName ?? u.loraId, weight: u.weight };
  });
  return {
    id: opts.id ?? "",
    name: name.trim(),
    family: opts.model?.familyId ?? null,
    modelId: opts.model?.id ?? null,
    civitaiVersionId: opts.model?.civitaiVersionId ?? null,
    styleId: c.styleId,
    shape: c.shape,
    quality: c.quality,
    stick: c.stick,
    count: c.count,
    fineTune,
    loras,
    builtin: false,
  };
}

export interface PresetApplication {
  patch: Partial<CreateParams>;
  /** The preset's model isn't installed (offer a download when it has a CivitAI version). */
  missingModel: { civitaiVersionId: number | null; family: string | null } | null;
  missingLoras: PresetLora[];
  /** The preset's style no longer exists. */
  missingStyle: boolean;
}

function snapshotBase(c: CreateParams): PresetBase {
  const { modelId, styleId, shape, quality, stick, count, loras, fineTune } = c;
  return { modelId, styleId, shape, quality, stick, count, loras, fineTune };
}

/** Choosing "None": drop the preset and restore what it overrode. Keeps the user's prompt and negative prompt. */
export function clearPreset(c: CreateParams): Partial<CreateParams> {
  const base = c.presetBase;
  if (!base) return { presetId: null, presetBase: null };
  const fineTune: FineTune = { ...base.fineTune };
  delete fineTune.negativePrompt;
  if (c.fineTune.negativePrompt) fineTune.negativePrompt = c.fineTune.negativePrompt;
  return { ...base, fineTune, presetId: null, presetBase: null };
}

/** Turn a preset into Create changes. Keeps the user's prompt and negative prompt. */
export function applyPreset(
  p: Preset,
  c: CreateParams,
  opts: { models: InstalledModel[]; loras: InstalledLora[]; styleIds: string[] },
): PresetApplication {
  const usable = opts.models.filter((m) => m.modes.includes("txt2img") && !m.isEditModel);
  const current = usable.find((m) => m.id === c.modelId) ?? null;
  let model: InstalledModel | null = null;
  if (p.modelId) model = usable.find((m) => m.id === p.modelId) ?? null;
  if (!model && p.civitaiVersionId != null) model = usable.find((m) => m.civitaiVersionId === p.civitaiVersionId) ?? null;
  if (!model && !p.modelId && p.civitaiVersionId == null) {
    // Family-only (or model-less) built-in preset: keep the current model when it fits.
    if (!p.family || current?.familyId === p.family) model = current;
    else model = usable.find((m) => m.familyId === p.family) ?? null;
  }
  const missingModel = model ? null : p.modelId || p.civitaiVersionId != null || p.family ? { civitaiVersionId: p.civitaiVersionId, family: p.family } : null;

  const fineTune: FineTune = {};
  for (const k of PRESET_FINE_TUNE_KEYS) {
    const v = p.fineTune?.[k];
    if (v !== undefined && v !== null) (fineTune as Record<string, unknown>)[k] = v;
  }
  if (c.fineTune.negativePrompt) fineTune.negativePrompt = c.fineTune.negativePrompt;

  const loras: LoraUse[] = [];
  const missingLoras: PresetLora[] = [];
  for (const pl of p.loras ?? []) {
    const l =
      (pl.loraId ? opts.loras.find((x) => x.id === pl.loraId) : undefined) ??
      (pl.civitaiVersionId != null ? opts.loras.find((x) => x.civitaiVersionId === pl.civitaiVersionId) : undefined);
    if (l) loras.push({ loraId: l.id, weight: pl.weight });
    else missingLoras.push(pl);
  }

  const missingStyle = !!p.styleId && !opts.styleIds.includes(p.styleId);
  const patch: Partial<CreateParams> = {
    presetId: p.id,
    // Keep the very first snapshot when hopping between presets, so None restores the original settings.
    presetBase: c.presetBase ?? snapshotBase(c),
    styleId: missingStyle ? c.styleId : p.styleId,
    fineTune,
    loras,
  };
  if (model) patch.modelId = model.id;
  if (p.shape) patch.shape = p.shape;
  if (p.quality) patch.quality = p.quality;
  patch.stick = p.stick ?? null;
  if (p.count === 1 || p.count === 2 || p.count === 4) patch.count = p.count;
  return { patch, missingModel, missingLoras, missingStyle };
}

// ------------------------------------------------------------------ display

/** One-line settings summary for a result card (never the prompt). */
export function settingsSummary(r: ResultImage): string {
  if (r.kind === "upscaled") {
    // Rust copies model/seed/sampling from the source (empty model id + seed 0 for an imported image).
    const parts = ["Upscaled", `${r.width}×${r.height}`];
    if (r.modelId) parts.push(`from ${r.modelLabel}`, `seed ${r.seed}`);
    return parts.join(" · ");
  }
  const parts = [r.modelLabel, `${r.width}×${r.height}`];
  if (r.steps > 0) parts.push(`${r.steps} steps`);
  if (r.guidance != null) parts.push(`guidance ${fmt(r.guidance)}`);
  if (r.cfg > 0 && (r.guidance == null || r.cfg !== 1)) parts.push(`CFG ${fmt(r.cfg)}`);
  const s = [r.sampler, r.scheduler].filter(Boolean).join(" ");
  if (s) parts.push(s);
  parts.push(`seed ${r.seed}`);
  return parts.join(" · ");
}

const fmt = (n: number) => String(Math.round(n * 100) / 100);
