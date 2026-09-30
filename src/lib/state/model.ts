// App state (pure): shape, initial values and the reducer.
//
// PRIVACY: this state holds prompt text (create.prompt, fineTune.negativePrompt,
// edit.instruction, edit.restylePrompt, describe.text, batch requests). It lives
// in memory only — never persist it (no localStorage/sessionStorage/IndexedDB),
// never log it, never put it in URLs. "Reset" (top bar) clears it.

import type {
  DescribeStyle,
  EngineStatus,
  FamilyUi,
  FineTune,
  GenerateRequest,
  GenerationProgress,
  GroupStatus,
  InstalledLora,
  InstalledModel,
  LoraUse,
  Preset,
  Quality,
  ResultImage,
  Settings,
  Shape,
  Style,
} from "../types";

export type TabId = "create" | "edit" | "describe" | "models";

/** An image held in the Rust session plus its in-memory blob: URL. */
export interface ImgRef {
  id: string;
  url: string;
  width: number;
  height: number;
}

/** Create fields a preset can override (and that choosing None can restore). */
export const PRESET_KEYS = ["modelId", "styleId", "shape", "quality", "stick", "count", "loras", "fineTune"] as const;
export type PresetKey = (typeof PRESET_KEYS)[number];
export type PresetSettings = Partial<Pick<CreateParams, PresetKey>>;
/** What the active preset changed: values before it (`before`) and the values it set (`applied`). */
export interface PresetBase {
  before: PresetSettings;
  applied: PresetSettings;
}

export interface CreateParams {
  modelId: string | null;
  presetId: string | null;
  /** What the active preset overrode, so choosing None can put it back. Null when no preset is active. */
  presetBase: PresetBase | null;
  /** prompt-bearing */
  prompt: string;
  styleId: string | null;
  shape: Shape;
  quality: Quality;
  /** "Stick to prompt" position 0…1; null = family default. */
  stick: number | null;
  count: 1 | 2 | 4;
  /** prompt-bearing (negativePrompt). Unset/null fields = registry default. */
  fineTune: FineTune;
  loras: LoraUse[];
}

/** The request behind a batch of results — used by "Variations". prompt-bearing, memory only. */
export interface Batch {
  id: string;
  request: GenerateRequest;
}

export type EditMode = "instruction" | "restyle";
export type ChangeAmount = "subtle" | "medium" | "strong";
export const CHANGE_STRENGTH: Record<ChangeAmount, number> = { subtle: 0.35, medium: 0.55, strong: 0.75 };

export interface EditNode {
  imageId: string;
  label: string;
  /** Settings of the edit that produced this node (null for the original). */
  meta: ResultImage | null;
}

export interface EditParams {
  chain: EditNode[];
  index: number;
  /** null = picked automatically. */
  mode: EditMode | null;
  /** prompt-bearing */
  instruction: string;
  /** prompt-bearing */
  restylePrompt: string;
  /** "Stay close to original" 0…1; null = family default. */
  stayClose: number | null;
  change: ChangeAmount;
  styleId: string | null;
  /** null = best installed edit model. */
  editModelId: string | null;
  /** Style add-ons (LoRAs) for edits; only those made for the edit's model are used. */
  loras: LoraUse[];
  /** Optional second image for "Describe a change" (image 2); session RAM like the chain. */
  secondImageId: string | null;
  /** null = the Create tab's model. */
  restyleModelId: string | null;
  quality: Quality;
  seed: number | null;
}

export interface DescribeParams {
  imageId: string | null;
  style: DescribeStyle;
  /** prompt-bearing (a description can be used as a prompt) */
  text: string;
}

export type JobKind = "create" | "edit" | "upscale" | "describe";

export interface Job {
  kind: JobKind;
  progress: GenerationProgress | null;
  startedAt: number;
  /** How many images the job makes (the strip's placeholders); not the current dial. */
  count?: number;
}

export interface Toast {
  id: number;
  text: string;
  tone?: "neutral" | "error";
  action?: { label: string; run: () => void };
}

export interface AppState {
  tab: TabId;
  settings: Settings | null;
  /** null until the first load. */
  models: InstalledModel[] | null;
  loras: InstalledLora[];
  styles: Style[];
  presets: Preset[];
  familyUi: Record<string, FamilyUi>;
  engine: EngineStatus | null;
  downloads: GroupStatus[];
  images: Record<string, ImgRef>;
  create: CreateParams;
  /** Newest first. */
  results: ResultImage[];
  selectedResultId: string | null;
  batches: Record<string, Batch>;
  /** result image id → batch id */
  resultBatch: Record<string, string>;
  edit: EditParams;
  describe: DescribeParams;
  job: Job | null;
  toasts: Toast[];
  /** Bumped by Reset; tab roots are keyed on it so local state resets too. */
  sessionNonce: number;
}

export const initialCreate = (): CreateParams => ({
  modelId: null,
  presetId: null,
  presetBase: null,
  prompt: "",
  styleId: null,
  shape: "square",
  quality: "balanced",
  stick: null,
  count: 1,
  fineTune: {},
  loras: [],
});

export const initialEdit = (): EditParams => ({
  chain: [],
  index: 0,
  mode: null,
  instruction: "",
  restylePrompt: "",
  stayClose: null,
  change: "medium",
  styleId: null,
  editModelId: null,
  loras: [],
  secondImageId: null,
  restyleModelId: null,
  quality: "balanced",
  seed: null,
});

export const initialDescribe = (): DescribeParams => ({ imageId: null, style: "sentence", text: "" });

export function initialState(): AppState {
  return {
    tab: "create",
    settings: null,
    models: null,
    loras: [],
    styles: [],
    presets: [],
    familyUi: {},
    engine: null,
    downloads: [],
    images: {},
    create: initialCreate(),
    results: [],
    selectedResultId: null,
    batches: {},
    resultBatch: {},
    edit: initialEdit(),
    describe: initialDescribe(),
    job: null,
    toasts: [],
    sessionNonce: 0,
  };
}

// ------------------------------------------------------------------ actions

export type Dial = "shape" | "quality" | "stick" | "count";

export type Action =
  | { type: "setTab"; tab: TabId }
  | { type: "setSettings"; settings: Settings }
  | { type: "setModels"; models: InstalledModel[] }
  | { type: "setLoras"; loras: InstalledLora[] }
  | { type: "setStyles"; styles: Style[] }
  | { type: "setPresets"; presets: Preset[] }
  | { type: "setFamilyUi"; ui: FamilyUi }
  | { type: "setEngine"; engine: EngineStatus }
  | { type: "download"; status: GroupStatus }
  | { type: "setDownloads"; list: GroupStatus[] }
  | { type: "clearFinishedDownloads" }
  | { type: "patchCreate"; patch: Partial<CreateParams> }
  | { type: "setFineTune"; patch: Partial<FineTune> }
  | { type: "setDial"; dial: "shape"; value: Shape }
  | { type: "setDial"; dial: "quality"; value: Quality }
  | { type: "setDial"; dial: "stick"; value: number | null }
  | { type: "setDial"; dial: "count"; value: 1 | 2 | 4 }
  | { type: "selectModel"; modelId: string | null }
  | { type: "keepLook"; on: boolean }
  | { type: "addResults"; batch: Batch | null; images: ResultImage[]; refs: ImgRef[] }
  | { type: "selectResult"; id: string | null }
  | { type: "removeResult"; id: string }
  | { type: "jobStart"; kind: JobKind; at: number; count?: number }
  | { type: "jobProgress"; progress: GenerationProgress }
  | { type: "jobEnd" }
  | { type: "editLoad"; ref: ImgRef }
  | { type: "editPush"; ref: ImgRef; meta?: ResultImage | null }
  | { type: "editGoto"; index: number }
  | { type: "editDelete"; index: number }
  | { type: "editClear" }
  | { type: "editSetSecond"; ref: ImgRef | null }
  | { type: "patchEdit"; patch: Partial<EditParams> }
  | { type: "describeLoad"; ref: ImgRef }
  | { type: "describeClear" }
  | { type: "patchDescribe"; patch: Partial<DescribeParams> }
  | { type: "toast"; toast: Toast }
  | { type: "dismissToast"; id: number }
  | { type: "clearSession" };

// ------------------------------------------------------------------ helpers

/** Models usable in the Create tab (text-to-image). */
export function createModels(models: InstalledModel[] | null): InstalledModel[] {
  return (models ?? []).filter((m) => m.modes.includes("txt2img") && !m.isEditModel);
}

/** Installed instruction-edit models. */
export function editModels(models: InstalledModel[] | null, twoImages = false): InstalledModel[] {
  return (models ?? []).filter((m) => (m.isEditModel || m.modes.includes("edit")) && (!twoImages || !!m.multiRef));
}

/** Most recently used ready model, else the first ready one, else the first one. */
export function pickDefaultModel(list: InstalledModel[]): InstalledModel | null {
  if (!list.length) return null;
  const ready = list.filter((m) => !m.missingComponents.length);
  const pool = ready.length ? ready : list;
  return [...pool].sort((a, b) => (b.lastUsed ?? 0) - (a.lastUsed ?? 0))[0];
}

/** Base architecture for LoRA compatibility (Pony/Illustrious LoRAs work on any SDXL). */
export function baseArch(familyId: string | null | undefined): string | null {
  if (!familyId) return null;
  if (familyId.startsWith("sdxl")) return "sdxl";
  if (familyId === "flux1_kontext") return "flux1";
  if (familyId.startsWith("flux1")) return "flux1";
  if (familyId.startsWith("z_image")) return "z_image";
  if (familyId.startsWith("qwen_image")) return "qwen_image";
  return familyId;
}

/** Strength a newly added style add-on starts at. */
export const DEFAULT_LORA_WEIGHT = 0.8;

/** Up to this many trigger words, all are added. A longer list is usually alternatives (one per
 *  character or outfit), so only the first is added until the user picks others on the chip. */
export const ALL_TRIGGER_WORDS_UP_TO = 3;

/** Trigger words this add-on adds to the prompt: the user's pick on the chip, else the default
 *  (none when "Add trigger words automatically" is off in Settings). */
export function pickedTriggerWords(u: LoraUse, lora: InstalledLora | undefined, autoAdd: boolean): string[] {
  const words = lora?.trainedWords ?? [];
  if (u.words) {
    const picked = new Set(u.words.map((w) => w.trim().toLowerCase()));
    return words.filter((w) => picked.has(w.trim().toLowerCase()));
  }
  if (!autoAdd) return [];
  return words.length <= ALL_TRIGGER_WORDS_UP_TO ? [...words] : words.slice(0, 1);
}

export function loraCompatible(lora: InstalledLora, modelFamily: string | null | undefined): boolean {
  if (!lora.familyId || !modelFamily) return true;
  return baseArch(lora.familyId) === baseArch(modelFamily);
}

/** Every session image id the UI still shows. */
export function referencedImageIds(s: Pick<AppState, "results" | "edit" | "describe">): Set<string> {
  const ids = new Set<string>();
  for (const r of s.results) ids.add(r.id);
  for (const n of s.edit.chain) ids.add(n.imageId);
  if (s.edit.secondImageId) ids.add(s.edit.secondImageId);
  if (s.describe.imageId) ids.add(s.describe.imageId);
  return ids;
}

/** Drop image refs nothing points at (their blob URLs get revoked by the store). */
function pruneImages(s: AppState): AppState {
  const keep = referencedImageIds(s);
  const keys = Object.keys(s.images);
  if (keys.every((k) => keep.has(k))) return s;
  const images: Record<string, ImgRef> = {};
  for (const k of keys) if (keep.has(k)) images[k] = s.images[k];
  const batches: Record<string, Batch> = {};
  const resultBatch: Record<string, string> = {};
  for (const r of s.results) {
    const b = s.resultBatch[r.id];
    if (b) {
      resultBatch[r.id] = b;
      if (s.batches[b]) batches[b] = s.batches[b];
    }
  }
  return { ...s, images, batches, resultBatch };
}

function withRefs(images: Record<string, ImgRef>, refs: ImgRef[]): Record<string, ImgRef> {
  const out = { ...images };
  for (const r of refs) out[r.id] = r;
  return out;
}

/** Remove keys whose value is undefined or null (= back to the registry default). */
export function compactFineTune(ft: FineTune): FineTune {
  const out: FineTune = {};
  for (const [k, v] of Object.entries(ft)) {
    if (v !== undefined && v !== null && !(typeof v === "string" && k !== "negativePrompt" && v === "")) {
      (out as Record<string, unknown>)[k] = v;
    }
  }
  return out;
}

const ACTIVE_DL = new Set(["queued", "downloading", "verifying"]);
export const isActiveDownload = (d: Pick<GroupStatus, "state">) => ACTIVE_DL.has(d.state);

// ------------------------------------------------------------------ reducer

export function reducer(s: AppState, a: Action): AppState {
  const next = inner(s, a);
  return next === s ? s : pruneImages(next);
}

function inner(s: AppState, a: Action): AppState {
  switch (a.type) {
    case "setTab":
      return s.tab === a.tab ? s : { ...s, tab: a.tab };
    case "setSettings":
      return { ...s, settings: a.settings };
    case "setModels": {
      const usable = createModels(a.models);
      let create = s.create;
      if (!create.modelId || !usable.some((m) => m.id === create.modelId)) {
        create = { ...create, modelId: pickDefaultModel(usable)?.id ?? null };
      }
      let edit = s.edit;
      if (edit.editModelId && !a.models.some((m) => m.id === edit.editModelId)) edit = { ...edit, editModelId: null };
      if (edit.restyleModelId && !a.models.some((m) => m.id === edit.restyleModelId)) edit = { ...edit, restyleModelId: null };
      return { ...s, models: a.models, create, edit };
    }
    case "setLoras":
      return { ...s, loras: a.loras };
    case "setStyles": {
      const exists = (id: string | null) => !id || a.styles.some((st) => st.id === id);
      return {
        ...s,
        styles: a.styles,
        create: exists(s.create.styleId) ? s.create : { ...s.create, styleId: null },
        edit: exists(s.edit.styleId) ? s.edit : { ...s.edit, styleId: null },
      };
    }
    case "setPresets":
      return {
        ...s,
        presets: a.presets,
        create: !s.create.presetId || a.presets.some((p) => p.id === s.create.presetId) ? s.create : { ...s.create, presetId: null, presetBase: null },
      };
    case "setFamilyUi":
      return { ...s, familyUi: { ...s.familyUi, [a.ui.familyId]: a.ui } };
    case "setEngine":
      return { ...s, engine: a.engine };
    case "download": {
      const i = s.downloads.findIndex((d) => d.groupId === a.status.groupId);
      const downloads = i >= 0 ? s.downloads.map((d, j) => (j === i ? a.status : d)) : [...s.downloads, a.status];
      return { ...s, downloads };
    }
    case "setDownloads": {
      // Keep finished ones we already know about (so "Done" stays visible until cleared).
      const merged = [...a.list];
      for (const d of s.downloads) if (!merged.some((m) => m.groupId === d.groupId)) merged.push(d);
      return { ...s, downloads: merged };
    }
    case "clearFinishedDownloads":
      return { ...s, downloads: s.downloads.filter(isActiveDownload) };

    case "patchCreate":
      return { ...s, create: { ...s.create, ...a.patch, ...("presetId" in a.patch && !a.patch.presetId && !("presetBase" in a.patch) ? { presetBase: null } : {}) } };
    case "setFineTune": {
      const ft = compactFineTune({ ...s.create.fineTune, ...a.patch });
      return { ...s, create: { ...s.create, fineTune: ft } };
    }
    case "setDial": {
      const ft = { ...s.create.fineTune };
      const c = { ...s.create };
      // Moving a simple dial hands control back to it: clear the matching Fine-tune override.
      if (a.dial === "shape") {
        c.shape = a.value;
        delete ft.width;
        delete ft.height;
      } else if (a.dial === "quality") {
        c.quality = a.value;
        delete ft.steps;
      } else if (a.dial === "stick") {
        c.stick = a.value;
        delete ft.cfg;
        delete ft.guidance;
      } else {
        c.count = a.value;
      }
      c.fineTune = ft;
      return { ...s, create: c };
    }
    case "selectModel": {
      if (a.modelId === s.create.modelId) return s;
      const models = s.models ?? [];
      const prev = models.find((m) => m.id === s.create.modelId);
      const nextModel = models.find((m) => m.id === a.modelId);
      let c: CreateParams = { ...s.create, modelId: a.modelId, presetId: null, presetBase: null };
      if (prev?.familyId !== nextModel?.familyId) {
        // Family-specific overrides don't carry over; the user's own text and seed do.
        const { negativePrompt, seed, vaeTiling } = c.fineTune;
        c = { ...c, stick: null, fineTune: compactFineTune({ negativePrompt, seed, vaeTiling }) };
      }
      return { ...s, create: c };
    }
    case "keepLook": {
      const sel = s.results.find((r) => r.id === s.selectedResultId);
      if (a.on) {
        const seed = sel?.seed ?? s.create.fineTune.seed ?? null;
        if (seed == null) return s;
        return { ...s, create: { ...s.create, fineTune: { ...s.create.fineTune, seed } } };
      }
      const ft = { ...s.create.fineTune };
      delete ft.seed;
      return { ...s, create: { ...s.create, fineTune: ft } };
    }
    case "addResults": {
      const batches = a.batch ? { ...s.batches, [a.batch.id]: a.batch } : s.batches;
      const resultBatch = { ...s.resultBatch };
      if (a.batch) for (const im of a.images) resultBatch[im.id] = a.batch.id;
      return {
        ...s,
        images: withRefs(s.images, a.refs),
        results: [...a.images, ...s.results.filter((r) => !a.images.some((n) => n.id === r.id))],
        selectedResultId: a.images[0]?.id ?? s.selectedResultId,
        batches,
        resultBatch,
      };
    }
    case "selectResult": {
      if (a.id === s.selectedResultId) return s;
      const prev = s.results.find((r) => r.id === s.selectedResultId);
      const next = s.results.find((r) => r.id === a.id);
      let create = s.create;
      // "Keep this look" follows the selection when the seed was locked from a result.
      if (prev && next && create.fineTune.seed != null && create.fineTune.seed === prev.seed) {
        create = { ...create, fineTune: { ...create.fineTune, seed: next.seed } };
      }
      return { ...s, selectedResultId: a.id, create };
    }
    case "removeResult": {
      const i = s.results.findIndex((r) => r.id === a.id);
      if (i < 0) return s;
      const results = s.results.filter((r) => r.id !== a.id);
      const selectedResultId = s.selectedResultId === a.id ? (results[Math.min(i, results.length - 1)]?.id ?? null) : s.selectedResultId;
      return { ...s, results, selectedResultId };
    }
    case "jobStart":
      return { ...s, job: { kind: a.kind, progress: null, startedAt: a.at, ...(a.count != null ? { count: a.count } : {}) } };
    case "jobProgress":
      return s.job ? { ...s, job: { ...s.job, progress: a.progress } } : s;
    case "jobEnd":
      return s.job ? { ...s, job: null } : s;

    case "editLoad":
      return {
        ...s,
        images: withRefs(s.images, [a.ref]),
        edit: { ...s.edit, chain: [{ imageId: a.ref.id, label: "Original", meta: null }], index: 0 },
      };
    case "editPush": {
      if (!s.edit.chain.length) return inner(s, { type: "editLoad", ref: a.ref });
      const kept = s.edit.chain.slice(0, s.edit.index + 1);
      const chain = [...kept, { imageId: a.ref.id, label: `Edit ${kept.length}`, meta: a.meta ?? null }];
      return { ...s, images: withRefs(s.images, [a.ref]), edit: { ...s.edit, chain, index: chain.length - 1 } };
    }
    case "editGoto": {
      const index = Math.max(0, Math.min(s.edit.chain.length - 1, a.index));
      return index === s.edit.index ? s : { ...s, edit: { ...s.edit, index } };
    }
    case "editDelete": {
      // The original (index 0) stays; use "New image" to start over.
      if (a.index <= 0 || a.index >= s.edit.chain.length) return s;
      const chain = s.edit.chain
        .filter((_, i) => i !== a.index)
        .map((n, i) => (i === 0 ? n : { ...n, label: `Edit ${i}` }));
      // Deleting the shown edit shows the one that took its place (the next edit), or the one before it if it was the last.
      const index = a.index < s.edit.index ? s.edit.index - 1 : a.index === s.edit.index ? Math.min(a.index, chain.length - 1) : s.edit.index;
      return { ...s, edit: { ...s.edit, chain, index } };
    }
    case "editClear":
      return { ...s, edit: { ...s.edit, chain: [], index: 0 } };
    case "editSetSecond":
      return a.ref
        ? { ...s, images: withRefs(s.images, [a.ref]), edit: { ...s.edit, secondImageId: a.ref.id } }
        : { ...s, edit: { ...s.edit, secondImageId: null } };
    case "patchEdit":
      return { ...s, edit: { ...s.edit, ...a.patch } };
    case "describeLoad":
      return { ...s, images: withRefs(s.images, [a.ref]), describe: { ...s.describe, imageId: a.ref.id, text: "" } };
    case "describeClear":
      return { ...s, describe: { ...s.describe, imageId: null, text: "" } };
    case "patchDescribe":
      return { ...s, describe: { ...s.describe, ...a.patch } };
    case "toast":
      return { ...s, toasts: [...s.toasts.slice(-3), a.toast] };
    case "dismissToast":
      return { ...s, toasts: s.toasts.filter((t) => t.id !== a.id) };
    case "clearSession": {
      const c = s.create;
      const { vaeTiling } = c.fineTune;
      return {
        ...s,
        images: {},
        create: { ...c, prompt: "", presetId: c.presetId, fineTune: compactFineTune({ vaeTiling }) },
        results: [],
        selectedResultId: null,
        batches: {},
        resultBatch: {},
        edit: { ...initialEdit(), mode: s.edit.mode, editModelId: s.edit.editModelId, restyleModelId: s.edit.restyleModelId, loras: s.edit.loras },
        describe: { ...initialDescribe(), style: s.describe.style },
        toasts: [],
        sessionNonce: s.sessionNonce + 1,
      };
    }
  }
}
