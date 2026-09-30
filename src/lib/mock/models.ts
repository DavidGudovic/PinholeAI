// Mock handlers for the models area (+ downloads). See ./index.ts.
// Browser-only fake backend: realistic data, simulated downloads with progress
// events, nothing persisted (RAM only, reset on reload).
//
// URL flags (for screenshots / manual testing): ?empty (nothing installed).
import type { MockTable } from "./index";
import { mockEmit } from "./index";
import type {
  AddFileResult,
  CoreError,
  DeletePreview,
  DownloadKind,
  FamilyChoice,
  Fit,
  GroupStatus,
  InstalledLora,
  InstalledModel,
  LinkedFolder,
  PastedResource,
  RecommendedPick,
  ResolvedResource,
  ResolvedResources,
  VramNeed,
} from "../types";
import { effectiveVramGb, mockFlags, mockRamGb, mockSettings } from "./app";
import { catalogEntryByVersion } from "./catalog";

const MB = 1024 * 1024;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const err = (code: string, message: string, details: string | null = null): CoreError => ({ code, message, details });

// ---------------------------------------------------------------- registry-ish data (mirrors config/models.yaml)
export interface MockFamily {
  label: string;
  components: string[];
  vram: { min: number; rec: number };
  license: string | null;
  edit?: boolean;
  modes: string[];
}

export const FAMILIES: Record<string, MockFamily> = {
  sd15: { label: "Stable Diffusion 1.5", components: [], vram: { min: 4, rec: 6 }, license: null, modes: ["txt2img", "img2img", "inpaint"] },
  sdxl: { label: "SDXL", components: ["sdxl_vae_fp16_fix"], vram: { min: 6, rec: 10 }, license: null, modes: ["txt2img", "img2img", "inpaint"] },
  sdxl_pony: { label: "SDXL · Pony", components: ["sdxl_vae_fp16_fix"], vram: { min: 6, rec: 10 }, license: null, modes: ["txt2img", "img2img", "inpaint"] },
  sdxl_illustrious: {
    label: "SDXL · Illustrious / NoobAI",
    components: ["sdxl_vae_fp16_fix"],
    vram: { min: 6, rec: 10 },
    license: null,
    modes: ["txt2img", "img2img", "inpaint"],
  },
  flux1_dev: { label: "FLUX.1 dev", components: ["flux_ae", "clip_l", "t5xxl_fp16"], vram: { min: 10, rec: 14 }, license: "Non-commercial license", modes: ["txt2img", "img2img"] },
  flux1_schnell: { label: "FLUX.1 schnell", components: ["flux_ae", "clip_l", "t5xxl_fp16"], vram: { min: 10, rec: 14 }, license: "Apache 2.0", modes: ["txt2img", "img2img"] },
  flux1_kontext: {
    label: "FLUX.1 Kontext",
    components: ["flux_ae", "clip_l", "t5xxl_fp8"],
    vram: { min: 6, rec: 10 },
    license: "Non-commercial license",
    edit: true,
    modes: ["edit"],
  },
  flux2_klein_4b: { label: "FLUX.2 klein 4B", components: [], vram: { min: 8, rec: 12 }, license: "Apache 2.0", modes: ["txt2img", "img2img", "edit"] },
  z_image_turbo: { label: "Z-Image Turbo", components: ["flux_ae", "qwen3_4b"], vram: { min: 12, rec: 16 }, license: "Apache 2.0", modes: ["txt2img", "img2img"] },
  qwen_image: { label: "Qwen-Image", components: ["qwen_image_vae", "qwen25_vl_7b_q8"], vram: { min: 12, rec: 16 }, license: "Apache 2.0", modes: ["txt2img", "img2img"] },
  qwen_image_edit_2511: {
    label: "Qwen Image Edit",
    components: ["qwen_image_vae", "qwen25_vl_7b_q8", "qwen25_vl_7b_mmproj"],
    vram: { min: 12, rec: 16 },
    license: "Apache 2.0",
    edit: true,
    modes: ["edit"],
  },
};

export const COMPONENTS: Record<string, { label: string; mb: number; path: string }> = {
  flux_ae: { label: "FLUX VAE", mb: 335, path: "models/vae/ae.safetensors" },
  clip_l: { label: "CLIP-L text encoder", mb: 246, path: "models/text_encoders/clip_l.safetensors" },
  t5xxl_fp16: { label: "T5-XXL text encoder", mb: 9790, path: "models/text_encoders/t5xxl_fp16.safetensors" },
  t5xxl_fp8: { label: "T5-XXL text encoder (compact)", mb: 4890, path: "models/text_encoders/t5xxl_fp8_e4m3fn.safetensors" },
  qwen3_4b: { label: "Qwen3 4B text encoder", mb: 8040, path: "models/text_encoders/qwen_3_4b.safetensors" },
  qwen_image_vae: { label: "Qwen-Image VAE", mb: 254, path: "models/vae/qwen_image_vae.safetensors" },
  qwen25_vl_7b_q8: { label: "Qwen2.5-VL 7B text encoder", mb: 8100, path: "models/text_encoders/Qwen2.5-VL-7B-Instruct-Q8_0.gguf" },
  qwen25_vl_7b_mmproj: { label: "Qwen2.5-VL vision adapter", mb: 850, path: "models/text_encoders/Qwen2.5-VL-7B-Instruct.mmproj-Q8_0.gguf" },
  sdxl_vae_fp16_fix: { label: "SDXL VAE (fp16 fix)", mb: 335, path: "models/vae/sdxl_vae_fp16_fix.safetensors" },
};

/**
 * Mirrors Rust `need_and_fit` (SPEC §6.2): the VRAM need against the GPU; without one,
 * the RAM for main file + components + activations against system RAM — SD 1.5 (or
 * ≤ 4 GB of weights) runs on the processor ("tight" = slow), everything else is too big.
 */
export function sizeFor(vram: VramNeed | null, familyId: string | null | undefined, mainBytes: number): { vram: VramNeed | null; fit: Fit | null } {
  if (!vram) return { vram: null, fit: null };
  if (effectiveVramGb() > 0) return { vram, fit: fitFor(vram) };
  const fam = familyId ? FAMILIES[familyId] : undefined;
  const weights = mainBytes + (fam?.components ?? []).reduce((a, c) => a + (COMPONENTS[c]?.mb ?? 0) * 1e6, 0);
  const gib = weights / 2 ** 30;
  const activation = familyId?.startsWith("sd15") ? 1 : 2;
  const gb = Math.ceil((gib + activation) * 10) / 10;
  const small = !!familyId?.startsWith("sd15") || gib <= 4;
  return { vram: { gb, minGb: gb, estimate: true, onCpu: true }, fit: small && gb + 4 <= mockRamGb() ? "tight" : "tooBig" };
}

/** Fits / Tight / Too big against the mock GPU (SPEC §6.2). */
export function fitFor(vram: VramNeed | null): Fit | null {
  if (!vram) return null;
  const have = effectiveVramGb();
  if (have <= 0) return "tooBig";
  if (vram.gb <= have - 1) return "fits";
  if (vram.minGb <= have) return "tight";
  return "tooBig";
}

// ---------------------------------------------------------------- installed state (RAM)
interface ModelRow extends Omit<InstalledModel, "fit" | "missingComponents"> {
  relPath: string;
}
interface LoraRow extends InstalledLora {
  relPath: string;
}

const HOUR = 3600_000;
const now = Date.now();
/** `lastUsed` is Unix SECONDS, as Rust sends it (installed.json). */
const secsAgo = (ms: number) => Math.floor((now - ms) / 1000);

const seedModels: ModelRow[] = [
  {
    id: "m_zimage",
    friendlyName: "Z-Image Turbo",
    familyId: "z_image_turbo",
    familyLabel: "Z-Image Turbo",
    styleBadge: "Realistic",
    modes: ["txt2img", "img2img"],
    isEditModel: false,
    sizeBytes: 12300 * MB,
    vram: { gb: 16, minGb: 12, estimate: false },
    lastUsed: secsAgo(2 * HOUR),
    licenseNote: "Apache 2.0",
    civitaiModelId: null,
    civitaiVersionId: null,
    baseModel: "ZImageTurbo",
    relPath: "models/diffusion/z_image_turbo_bf16.safetensors",
  },
  {
    id: "m_jugg",
    friendlyName: "Juggernaut XL",
    familyId: "sdxl",
    familyLabel: "SDXL",
    styleBadge: "Realistic",
    modes: ["txt2img", "img2img", "inpaint"],
    isEditModel: false,
    sizeBytes: 6776 * MB,
    vram: { gb: 9.5, minGb: 6, estimate: true },
    lastUsed: secsAgo(3 * 24 * HOUR),
    licenseNote: "CreativeML Open RAIL++-M",
    civitaiModelId: 133005,
    civitaiVersionId: 782002,
    baseModel: "SDXL 1.0",
    relPath: "models/checkpoints/juggernautXL_ragnarok.safetensors",
  },
  {
    id: "m_wai",
    friendlyName: "WAI-Illustrious-SDXL",
    familyId: "sdxl_illustrious",
    familyLabel: "SDXL · Illustrious / NoobAI",
    styleBadge: "Anime",
    modes: ["txt2img", "img2img", "inpaint"],
    isEditModel: false,
    sizeBytes: 6938 * MB,
    vram: { gb: 9.5, minGb: 6, estimate: true },
    lastUsed: null,
    licenseNote: "Illustrious license",
    civitaiModelId: 827184,
    civitaiVersionId: 1410435,
    baseModel: "Illustrious",
    relPath: "models/checkpoints/waiIllustriousSDXL_v140.safetensors",
  },
  {
    id: "m_fluxdev",
    friendlyName: "FLUX.1 dev (Q8)",
    familyId: "flux1_dev",
    familyLabel: "FLUX.1 dev",
    styleBadge: "Realistic",
    modes: ["txt2img", "img2img"],
    isEditModel: false,
    sizeBytes: 12110 * MB,
    vram: { gb: 14, minGb: 10, estimate: true },
    lastUsed: secsAgo(40 * 24 * HOUR),
    licenseNote: "Non-commercial license",
    civitaiModelId: 618692,
    civitaiVersionId: 691639,
    baseModel: "Flux.1 D",
    relPath: "models/diffusion/flux1-dev-Q8_0.gguf",
  },
];

const seedLoras: LoraRow[] = [
  {
    id: "l_film",
    friendlyName: "Film Photography Style XL",
    familyId: "sdxl",
    baseModel: "SDXL 1.0",
    trainedWords: ["film photo", "kodak portra 400"],
    sizeBytes: 228 * MB,
    civitaiModelId: 991000,
    civitaiVersionId: 991001,
    relPath: "models/loras/film_photography_xl.safetensors",
  },
  {
    id: "l_detail",
    friendlyName: "Detail Tweaker XL",
    familyId: "sdxl",
    baseModel: "SDXL 1.0",
    trainedWords: [],
    sizeBytes: 218 * MB,
    civitaiModelId: 135867,
    civitaiVersionId: 135867,
    relPath: "models/loras/add-detail-xl.safetensors",
  },
];

const empty = () => mockFlags().empty;
let models: ModelRow[] | null = null;
let loras: LoraRow[] | null = null;
let components: Set<string> | null = null;
// ?captioner → the Describe model is installed (as in describe.ts; screenshots).
let captionerInstalled = typeof location !== "undefined" && new URLSearchParams(location.search).has("captioner");

function state() {
  if (!models) {
    models = empty() ? [] : seedModels.map((m) => ({ ...m }));
    // `?flux2`: a FLUX.2 klein model (takes a reference picture in Create, edits in Edit).
    if (!empty() && mockFlags().flux2) models.unshift({ ...seedModels[0], id: "m_klein", friendlyName: "FLUX.2 klein 4B", familyId: "flux2_klein_4b", familyLabel: "FLUX.2 klein 4B", modes: ["txt2img", "img2img", "edit"], multiRef: true, lastUsed: secsAgo(60_000), civitaiModelId: null, civitaiVersionId: null, baseModel: null, relPath: "models/diffusion/flux-2-klein-4b.safetensors" });
    loras = empty() ? [] : seedLoras.map((l) => ({ ...l }));
    components = new Set(empty() ? [] : ["flux_ae", "clip_l", "qwen3_4b", "sdxl_vae_fp16_fix"]);
  }
  return { models: models!, loras: loras!, components: components! };
}

export function installedComponents(): Set<string> {
  return state().components;
}

export function isVersionInstalled(versionId: number): boolean {
  const s = state();
  return s.models.some((m) => m.civitaiVersionId === versionId) || s.loras.some((l) => l.civitaiVersionId === versionId);
}

function missingFor(familyId: string | null): string[] {
  if (!familyId || !FAMILIES[familyId]) return [];
  const have = installedComponents();
  return FAMILIES[familyId].components.filter((c) => !have.has(c));
}

function toInstalled(m: ModelRow): InstalledModel {
  const { relPath: _relPath, ...rest } = m;
  void _relPath;
  const sized = sizeFor(m.vram, m.familyId, m.sizeBytes);
  return { ...rest, vram: sized.vram, fit: sized.fit, missingComponents: missingFor(m.familyId).map((c) => COMPONENTS[c].label) };
}

function toLora(l: LoraRow): InstalledLora {
  const { relPath: _relPath, ...rest } = l;
  void _relPath;
  return rest;
}

export function registerModel(row: Omit<ModelRow, "id" | "lastUsed">): InstalledModel {
  const s = state();
  const m: ModelRow = { ...row, id: `m_${Math.random().toString(36).slice(2, 8)}`, lastUsed: null };
  s.models.push(m);
  if (m.familyId) for (const c of FAMILIES[m.familyId]?.components ?? []) s.components.add(c);
  return toInstalled(m);
}

export function registerLora(row: Omit<LoraRow, "id">): InstalledLora {
  const s = state();
  const l: LoraRow = { ...row, id: `l_${Math.random().toString(36).slice(2, 8)}` };
  s.loras.push(l);
  return toLora(l);
}

export const modelsChanged = () => mockEmit("models-changed", null);

/** Rust sets `lastUsed` (Unix seconds) after every successful generation. */
export function touchLastUsed(modelId: string) {
  const m = state().models.find((x) => x.id === modelId);
  if (m) m.lastUsed = Math.floor(Date.now() / 1000);
}

// ---------------------------------------------------------------- download simulator (shared with app.ts / catalog.ts)
export interface MockFile {
  name: string;
  bytes: number;
}

interface DownloadOpts {
  /** GroupStatus.kind, as Rust tags it. Default "model". */
  kind?: DownloadKind;
  /** Total simulated time. Default scales mildly with size (8–16 s). */
  durationMs?: number;
  /** Fail when this fraction is reached (0…1). */
  failAt?: number;
  onDone?: () => void;
  onFail?: (e: CoreError) => void;
  onCancel?: () => void;
}

const downloads = new Map<string, GroupStatus>();
const running = new Map<string, { timer: ReturnType<typeof setInterval>; opts: DownloadOpts }>();
let seq = 0;

function publish(g: GroupStatus) {
  downloads.set(g.groupId, g);
  mockEmit("download-progress", g);
}

export function startMockDownload(label: string, files: MockFile[], opts: DownloadOpts = {}): string {
  const groupId = `dl-${(++seq).toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
  const total = files.reduce((a, f) => a + f.bytes, 0);
  const bounds: number[] = [];
  files.reduce((acc, f) => {
    bounds.push(acc + f.bytes);
    return acc + f.bytes;
  }, 0);
  let g: GroupStatus = {
    groupId,
    label,
    kind: opts.kind ?? "model",
    state: "queued",
    currentFile: files[0]?.name ?? null,
    fileIndex: 0,
    fileCount: files.length,
    downloadedBytes: 0,
    totalBytes: total,
    error: null,
  };
  publish(g);
  const duration = opts.durationMs ?? Math.min(16000, 8000 + (total / (1024 * MB)) * 350);
  const tick = 200;
  let verifyTicks = 0;
  const timer = setInterval(() => {
    if (g.state === "queued") {
      g = { ...g, state: "downloading" };
      publish(g);
      return;
    }
    if (g.state === "downloading") {
      const step = (total * tick) / duration;
      const next = Math.min(total, g.downloadedBytes + step * (0.6 + Math.random() * 0.8));
      const idx = Math.max(0, bounds.findIndex((b) => next < b));
      const fileIndex = next >= total ? files.length - 1 : idx;
      if (opts.failAt != null && next / total >= opts.failAt) {
        clearInterval(timer);
        running.delete(groupId);
        const e = err(
          "network",
          "The download stopped because the connection was lost. Check your internet connection and try again.",
          `GET ${files[fileIndex]?.name}: connection reset by peer (os error 104) after ${Math.round(next / MB)} MB`,
        );
        g = { ...g, downloadedBytes: next, state: "failed", error: e.message };
        publish(g);
        opts.onFail?.(e);
        return;
      }
      g = { ...g, downloadedBytes: next, fileIndex, currentFile: files[fileIndex]?.name ?? null, state: next >= total ? "verifying" : "downloading" };
      publish(g);
      return;
    }
    if (g.state === "verifying") {
      verifyTicks += 1;
      if (verifyTicks >= 4) {
        clearInterval(timer);
        running.delete(groupId);
        g = { ...g, state: "done", currentFile: null };
        publish(g);
        opts.onDone?.();
      }
    }
  }, tick);
  running.set(groupId, { timer, opts });
  return groupId;
}

export function cancelMockDownload(groupId: string) {
  const r = running.get(groupId);
  const g = downloads.get(groupId);
  if (!r || !g) return;
  clearInterval(r.timer);
  running.delete(groupId);
  publish({ ...g, state: "cancelled", currentFile: null });
  r.opts.onCancel?.();
}

// ---------------------------------------------------------------- recommended (SPEC §6.1)
interface RecCandidate {
  role: string;
  roleLabel: string;
  title: string;
  familyId: string;
  goodAt: string;
  quant: string | null;
  mainMb: number;
  vram: { min: number; rec: number };
  licenseNote: string | null;
  isEdit?: boolean;
}

function pickFor(role: string): RecCandidate | null {
  const v = effectiveVramGb();
  switch (role) {
    case "realistic": {
      const base = {
        role,
        roleLabel: "Realistic",
        title: "Z-Image Turbo",
        familyId: "z_image_turbo",
        goodAt: "Photos and realistic scenes in seconds. Understands long, natural sentences.",
        licenseNote: "Apache 2.0 — fine for client work",
      };
      if (v >= 12) return { ...base, quant: "bf16", mainMb: 12300, vram: { min: 12, rec: 16 } };
      if (v >= 8) return { ...base, quant: "q8_0", mainMb: 7200, vram: { min: 8, rec: 12 } };
      if (v >= 5) return { ...base, quant: "q4_k", mainMb: 4000, vram: { min: 5, rec: 8 } };
      // Like config/models.yaml: the last realistic candidate, for no GPU (sized against RAM) or < 5 GB.
      if (v >= 4 || v <= 0)
        return {
          ...base,
          title: "Stable Diffusion 1.5 (small, runs on any computer)",
          familyId: "sd15",
          goodAt: "Photos and lifelike pictures",
          quant: "fp16",
          mainMb: 2034,
          vram: { min: 4, rec: 6 },
          licenseNote: null,
        };
      return null;
    }
    case "anime":
      if (v < 6) return null;
      return {
        role,
        roleLabel: "Anime",
        title: "WAI-Illustrious-SDXL",
        familyId: "sdxl_illustrious",
        goodAt: "Anime and manga art with clean lines and vivid colour. Works well with tags.",
        quant: null,
        mainMb: 6938,
        vram: { min: 6, rec: 10 },
        licenseNote: "Illustrious license — check before commercial use",
      };
    case "edit":
      if (v < 6) return null;
      if (v >= 12)
        return {
          role,
          roleLabel: "Edit",
          title: "Qwen Image Edit 2511",
          familyId: "qwen_image_edit_2511",
          goodAt: "Change a photo by describing it — objects, backgrounds, lighting or style.",
          quant: "q4_k",
          mainMb: 13100,
          vram: { min: 12, rec: 16 },
          licenseNote: "Apache 2.0 — fine for client work",
          isEdit: true,
        };
      return {
        role,
        roleLabel: "Edit",
        title: "FLUX.1 Kontext",
        familyId: "flux1_kontext",
        goodAt: "Edit a picture by describing the change. Lighter on graphics memory.",
        quant: "q4_k",
        mainMb: 6900,
        vram: { min: 6, rec: 10 },
        licenseNote: "Non-commercial license",
        isEdit: true,
      };
    default:
      return null;
  }
}

function recommendedFor(role: string): RecommendedPick {
  const s = state();
  const roleLabel = { realistic: "Realistic", anime: "Anime", edit: "Edit", describe: "Describe" }[role] ?? role;
  if (role === "describe") {
    const editInstalled = s.models.some((m) => m.familyId === "qwen_image_edit_2511");
    if (editInstalled)
      return {
        role,
        roleLabel,
        title: "Qwen2.5-VL 7B (from your edit model)",
        familyId: null,
        goodAt: "Turns any image into a prompt you can reuse. Nothing extra to download.",
        downloadBytes: 0,
        vram: { gb: 9, minGb: 8, estimate: false },
        fit: fitFor({ gb: 9, minGb: 8, estimate: false }),
        installed: true,
        quant: null,
        licenseNote: "Apache 2.0",
        unavailableReason: null,
        note: null,
      };
    const vram = { gb: 4, minGb: 3, estimate: false };
    return {
      role,
      roleLabel,
      title: "Qwen2.5-VL 3B describer",
      familyId: null,
      goodAt: "Turns any image into a prompt you can reuse — as a sentence or as tags.",
      downloadBytes: captionerInstalled ? 0 : (1900 + 850) * MB,
      vram,
      fit: fitFor(vram),
      installed: captionerInstalled,
      quant: "q4_k",
      licenseNote: "Qwen Research License — personal use",
      unavailableReason: null,
      note: null,
    };
  }
  const c = pickFor(role);
  if (!c)
    return {
      role,
      roleLabel,
      title: null,
      familyId: null,
      goodAt: null,
      downloadBytes: 0,
      vram: null,
      fit: null,
      installed: false,
      quant: null,
      licenseNote: null,
      unavailableReason: tooBigReason(role),
      note: null,
    };
  const installed = s.models.some((m) => m.familyId === c.familyId && m.friendlyName.startsWith(c.title));
  const missing = FAMILIES[c.familyId]?.components.filter((id) => !s.components.has(id)) ?? [];
  const bytes = installed ? 0 : c.mainMb * MB + missing.reduce((a, id) => a + COMPONENTS[id].mb * MB, 0);
  const sized = sizeFor({ gb: c.vram.rec, minGb: c.vram.min, estimate: false }, c.familyId, c.mainMb * MB);
  return {
    role,
    roleLabel: c.roleLabel,
    title: c.title,
    familyId: c.familyId,
    goodAt: c.goodAt,
    downloadBytes: bytes,
    vram: sized.vram,
    fit: sized.fit,
    installed,
    quant: c.quant,
    licenseNote: c.licenseNote,
    unavailableReason: null,
    note: null,
  };
}

/** Same words as Rust recommend.rs `too_big_reason` (no "go to the Models tab": the cards show there too). */
function tooBigReason(role: string): string {
  const v = effectiveVramGb();
  if (v > 0) return `None of the recommended models fit in ${v} GB of graphics memory. Smaller ones are available when you browse CivitAI.`;
  if (role === "edit")
    return "Editing by description needs a graphics card, and Pinhole didn't find one it can use. Restyle still works with a Create model, slowly, on the processor.";
  return `The recommended ${role} models need a graphics card, and Pinhole didn't find one it can use. Only small models such as Stable Diffusion 1.5 run on the processor, slowly.`;
}

const activeRecommended = new Map<string, string>();

async function installRecommended(role: string) {
  await sleep(250);
  if (mockSettings().offline)
    throw err("offline", "Offline mode is on. Turn it off in Settings to download models.");
  const existing = activeRecommended.get(role);
  if (existing && downloads.get(existing) && ["queued", "downloading", "verifying"].includes(downloads.get(existing)!.state))
    return { groupId: existing };
  const pick = recommendedFor(role);
  if (pick.installed) throw err("invalid", "This model is already installed.");
  if (pick.unavailableReason) throw err("vram", pick.unavailableReason);
  if (role === "describe") {
    const groupId = startMockDownload("Describe model", [
      { name: "Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf", bytes: 1900 * MB },
      { name: "mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf", bytes: 850 * MB },
    ], {
      kind: "captioner",
      onDone: () => {
        captionerInstalled = true;
        modelsChanged();
      },
    });
    activeRecommended.set(role, groupId);
    return { groupId };
  }
  const c = pickFor(role)!;
  const s = state();
  const missing = FAMILIES[c.familyId].components.filter((id) => !s.components.has(id));
  const mainName =
    c.familyId === "sd15"
      ? "v1-5-pruned-emaonly-fp16.safetensors"
      : c.familyId === "z_image_turbo"
      ? c.quant === "bf16"
        ? "z_image_turbo_bf16.safetensors"
        : `z_image_turbo-${c.quant?.toUpperCase()}.gguf`
      : c.familyId === "qwen_image_edit_2511"
        ? "qwen-image-edit-2511-Q4_K_M.gguf"
        : c.familyId === "flux1_kontext"
          ? "flux1-kontext-dev-Q4_K_M.gguf"
          : `${c.title.replace(/[^A-Za-z0-9]+/g, "_")}.safetensors`;
  const files: MockFile[] = [{ name: mainName, bytes: c.mainMb * MB }, ...missing.map((id) => ({ name: COMPONENTS[id].path.split("/").pop()!, bytes: COMPONENTS[id].mb * MB }))];
  const groupId = startMockDownload(c.title, files, {
    onDone: () => {
      registerModel({
        friendlyName: c.title,
        familyId: c.familyId,
        familyLabel: FAMILIES[c.familyId].label,
        // Rust gives the badge only to the family that heads the role (not the SD 1.5 fallback).
        styleBadge: role === "anime" ? "Anime" : role === "realistic" && c.familyId !== "sd15" ? "Realistic" : null,
        modes: FAMILIES[c.familyId].modes,
        isEditModel: !!c.isEdit,
        sizeBytes: c.mainMb * MB,
        vram: { gb: c.vram.rec, minGb: c.vram.min, estimate: false },
        licenseNote: c.licenseNote,
        civitaiModelId: null,
        civitaiVersionId: null,
        baseModel: null,
        relPath: `models/${c.familyId.startsWith("sdxl") || c.familyId === "sd15" ? "checkpoints" : "diffusion"}/${mainName}`,
      });
      modelsChanged();
    },
  });
  activeRecommended.set(role, groupId);
  return { groupId };
}

// ---------------------------------------------------------------- add a file I already have
const pendingChoices = new Map<string, { path: string; candidates: FamilyChoice[] }>();

const choice = (familyId: string): FamilyChoice => ({ familyId, label: FAMILIES[familyId].label });

function registerLocal(path: string, familyId: string): AddFileResult {
  const fileName = path.split(/[\\/]/).pop() ?? path;
  const nice = fileName.replace(/\.(safetensors|gguf)$/i, "").replace(/[_-]+/g, " ");
  const fam = FAMILIES[familyId];
  const model = registerModel({
    friendlyName: nice,
    familyId,
    familyLabel: fam.label,
    styleBadge: null,
    modes: fam.modes,
    isEditModel: !!fam.edit,
    sizeBytes: (familyId.startsWith("flux") ? 11900 : 6600) * MB,
    vram: { gb: fam.vram.rec, minGb: fam.vram.min, estimate: true },
    licenseNote: fam.license,
    civitaiModelId: null,
    civitaiVersionId: null,
    baseModel: null,
    relPath: `models/${familyId.startsWith("sdxl") || familyId === "sd15" ? "checkpoints" : "diffusion"}/${fileName}`,
  });
  modelsChanged();
  return { model, lora: null, needsChoice: null };
}

async function addLocalModel(path: string): Promise<AddFileResult> {
  await sleep(1400);
  const fileName = path.split(/[\\/]/).pop() ?? path;
  const lower = fileName.toLowerCase();
  if (!/\.(safetensors|gguf)$/.test(lower))
    throw err("invalid", "Pinhole can only add .safetensors and .gguf files. Older .ckpt/.pt files can hide harmful code.");
  if (lower.includes("broken"))
    throw err("invalid", "This file doesn't look like a model Pinhole can read. It may be damaged — try downloading it again.", "safetensors header: invalid JSON at byte 1834");
  if (lower.includes("lora")) {
    const lora = registerLora({
      friendlyName: fileName.replace(/\.(safetensors|gguf)$/i, "").replace(/[_-]+/g, " "),
      familyId: "sdxl",
      baseModel: null,
      trainedWords: [],
      sizeBytes: 170 * MB,
      civitaiModelId: null,
      civitaiVersionId: null,
      relPath: `models/loras/${fileName}`,
    });
    modelsChanged();
    return { model: null, lora, needsChoice: null };
  }
  let candidates: FamilyChoice[] | null = null;
  if (lower.includes("flux") || lower.includes("kontext")) candidates = [choice("flux1_dev"), choice("flux1_schnell"), choice("flux1_kontext")];
  else if (lower.includes("qwen")) candidates = [choice("qwen_image"), choice("qwen_image_edit_2511")];
  else if (lower.includes("pony") || lower.includes("illustrious")) candidates = [choice("sdxl"), choice("sdxl_pony"), choice("sdxl_illustrious")];
  if (candidates) {
    const token = `tok_${Math.random().toString(36).slice(2, 10)}`;
    pendingChoices.set(token, { path, candidates });
    return { model: null, lora: null, needsChoice: { token, fileName, candidates } };
  }
  return registerLocal(path, "sdxl");
}

// ---------------------------------------------------------------- delete
function orphanComponents(modelId: string): string[] {
  const s = state();
  const target = s.models.find((m) => m.id === modelId);
  if (!target?.familyId) return [];
  const others = s.models.filter((m) => m.id !== modelId);
  const used = new Set(others.flatMap((m) => (m.familyId ? FAMILIES[m.familyId]?.components ?? [] : [])));
  return (FAMILIES[target.familyId]?.components ?? []).filter((c) => s.components.has(c) && !used.has(c));
}

function previewDelete(modelId: string): DeletePreview {
  const s = state();
  const m = s.models.find((x) => x.id === modelId);
  if (m)
    return {
      modelId,
      files: [
        { relPath: m.relPath, sizeBytes: m.sizeBytes, reason: "model" },
        ...orphanComponents(modelId).map((c) => ({ relPath: COMPONENTS[c].path, sizeBytes: COMPONENTS[c].mb * MB, reason: "orphanComponent" as const })),
      ],
    };
  const l = s.loras.find((x) => x.id === modelId);
  if (l) return { modelId, files: [{ relPath: l.relPath, sizeBytes: l.sizeBytes, reason: "model" }] };
  throw err("not_found", "That model is no longer installed.");
}

// ---------------------------------------------------------------- paste from CivitAI
function resolveOne(r: PastedResource): ResolvedResource {
  const s = state();
  const byVersion = r.modelVersionId != null ? r.modelVersionId : null;
  const inst = byVersion != null ? s.models.find((m) => m.civitaiVersionId === byVersion) ?? s.loras.find((l) => l.civitaiVersionId === byVersion) : undefined;
  const entry = byVersion != null ? catalogEntryByVersion(byVersion) : null;
  const displayName = inst?.friendlyName ?? entry?.name ?? r.modelName ?? "Unknown model";
  return {
    resource: r,
    installedId: inst?.id ?? null,
    installableVersionId: !inst && entry && !entry.blockedReason ? entry.versionId : null,
    displayName,
    familyId: inst?.familyId ?? entry?.familyId ?? null,
    downloadBytes: !inst && entry ? entry.downloadBytes : null,
    fit: !inst && entry ? entry.fit : null,
    problem: !inst && !entry ? "Couldn't find this on CivitAI." : entry?.blockedReason ?? null,
  };
}

// ---------------------------------------------------------------- other apps' models folders
let linkedFolders: LinkedFolder[] = [];

function linkedView(f: LinkedFolder): LinkedFolder {
  const s = state();
  const models = s.models.filter((m) => m.linkedFolder === f.name).length;
  const addons = s.loras.filter((l) => l.linkedFolder === f.name).length;
  return { ...f, models, addons };
}

async function addLinkedFolder(path: string): Promise<LinkedFolder> {
  await sleep(200);
  if (linkedFolders.some((f) => f.path === path)) throw err("invalid", "You've already added this folder.");
  const name = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  const id = Math.random().toString(36).slice(2, 10);
  const folder: LinkedFolder = { id, path, name, available: true, scanning: true, models: 0, addons: 0, parts: 0, notUsed: 0 };
  linkedFolders.push(folder);
  setTimeout(() => {
    const s = state();
    const base = { styleBadge: null, isEditModel: false, lastUsed: null, civitaiModelId: null, civitaiVersionId: null, baseModel: null, licenseNote: null, linkedFolder: name };
    s.models.push(
      { ...base, id: `m_${id}_1`, friendlyName: "juggernautXL v9", familyId: "sdxl", familyLabel: FAMILIES.sdxl.label, modes: FAMILIES.sdxl.modes, sizeBytes: 6600 * MB, vram: { gb: 10, minGb: 6, estimate: true }, relPath: `linked/${id}/checkpoints/juggernautXL_v9.safetensors` },
      { ...base, id: `m_${id}_2`, friendlyName: "ponyDiffusionV6XL", familyId: "sdxl_pony", familyLabel: FAMILIES.sdxl_pony.label, modes: FAMILIES.sdxl_pony.modes, sizeBytes: 6600 * MB, vram: { gb: 10, minGb: 6, estimate: true }, relPath: `linked/${id}/checkpoints/ponyDiffusionV6XL.safetensors` },
      { ...base, id: `m_${id}_3`, friendlyName: "flux1 dev fp8", familyId: "flux1_dev", familyLabel: FAMILIES.flux1_dev.label, modes: FAMILIES.flux1_dev.modes, sizeBytes: 11900 * MB, vram: { gb: 14, minGb: 10, estimate: true }, licenseNote: FAMILIES.flux1_dev.license, relPath: `linked/${id}/unet/flux1-dev-fp8.safetensors` },
    );
    s.loras.push({ id: `l_${id}_1`, friendlyName: "Watercolor wash", familyId: "sdxl", baseModel: "SDXL 1.0", trainedWords: ["watercolor"], sizeBytes: 220 * MB, civitaiModelId: null, civitaiVersionId: null, linkedFolder: name, relPath: `linked/${id}/loras/watercolor.safetensors` });
    const f = linkedFolders.find((x) => x.id === id);
    if (f) Object.assign(f, { scanning: false, parts: 1, notUsed: 4 });
    modelsChanged();
  }, 1500);
  modelsChanged();
  return linkedView(folder);
}

// ---------------------------------------------------------------- table
const table: MockTable = {
  list_models: async () => {
    await sleep(120);
    return state().models.map(toInstalled);
  },
  list_loras: async () => {
    await sleep(80);
    return state().loras.map(toLora);
  },
  set_lora_trigger_words: async (a) => {
    await sleep(120);
    const l = state().loras.find((x) => x.id === String(a.loraId));
    if (!l) throw err("not_found", "That add-on isn't installed any more.");
    const out: string[] = [];
    for (const w of (a.words as string[]).map((x) => x.trim())) if (w && !out.some((o) => o.toLowerCase() === w.toLowerCase())) out.push(w);
    l.trainedWords = out;
    return toLora(l);
  },
  list_helpers: async () => {
    await sleep(60);
    return captionerInstalled ? [{ id: "describe", friendlyName: "Describe model", purpose: "describe", sizeBytes: (1930 + 845) * 1_000_000 }] : [];
  },
  delete_helper: async (a) => {
    await sleep(300);
    if (String(a.helperId) !== "describe" || !captionerInstalled) throw err("not_found", "That helper isn't installed any more.");
    captionerInstalled = false;
    modelsChanged();
  },
  open_models_folder: async () => undefined,
  get_recommended: async () => {
    await sleep(250);
    return ["realistic", "anime", "edit", "describe"].map(recommendedFor);
  },
  install_recommended: (a) => installRecommended(String(a.role)),
  add_local_model: (a) => addLocalModel(String(a.path)),
  confirm_family: async (a) => {
    await sleep(900);
    const p = pendingChoices.get(String(a.token));
    if (!p) throw err("not_found", "Please add the file again.");
    pendingChoices.delete(String(a.token));
    return registerLocal(p.path, String(a.familyId));
  },
  install_missing_parts: async (a) => {
    await sleep(200);
    const s = state();
    const m = s.models.find((x) => x.id === String(a.modelId));
    if (!m?.familyId) throw err("not_found", "That model isn't installed any more.");
    const missing = missingFor(m.familyId);
    if (!missing.length) throw err("invalid", "This model already has every part it needs.");
    const groupId = startMockDownload(`Parts for ${m.friendlyName}`, missing.map((id) => ({ name: COMPONENTS[id].path.split("/").pop()!, bytes: COMPONENTS[id].mb * MB })), {
      onDone: () => {
        for (const id of missing) s.components.add(id);
        modelsChanged();
      },
    });
    return { groupId };
  },
  list_linked_folders: async () => {
    await sleep(60);
    return linkedFolders.map(linkedView);
  },
  add_linked_folder: (a) => addLinkedFolder(String(a.path)),
  remove_linked_folder: async (a) => {
    await sleep(150);
    const f = linkedFolders.find((x) => x.id === String(a.id));
    if (!f) throw err("not_found", "That folder isn't in the list any more.");
    linkedFolders = linkedFolders.filter((x) => x !== f);
    models = state().models.filter((m) => m.linkedFolder !== f.name);
    loras = state().loras.filter((l) => l.linkedFolder !== f.name);
    modelsChanged();
  },
  rescan_linked_folders: async () => {
    await sleep(100);
    modelsChanged();
  },
  preview_delete: async (a) => {
    await sleep(150);
    return previewDelete(String(a.modelId));
  },
  delete_model: async (a) => {
    await sleep(400);
    const s = state();
    const id = String(a.modelId);
    const orphans = orphanComponents(id);
    const before = s.models.length + s.loras.length;
    models = s.models.filter((m) => m.id !== id);
    loras = s.loras.filter((l) => l.id !== id);
    if (models.length + loras.length === before) throw err("not_found", "That model is no longer installed.");
    for (const c of orphans) s.components.delete(c);
    modelsChanged();
  },
  resolve_civitai_resources: async (a): Promise<ResolvedResources> => {
    await sleep(300);
    const list = (a.resources as PastedResource[]) ?? [];
    const out: ResolvedResources = { checkpoint: null, loras: [], ignored: [] };
    for (const r of list) {
      const t = r.type.toLowerCase();
      if (t === "checkpoint" && !out.checkpoint) out.checkpoint = resolveOne(r);
      else if (t === "lora" || t === "locon") out.loras.push(resolveOne(r));
      else out.ignored.push({ ...resolveOne(r), problem: "Pinhole doesn't use this kind of resource." });
    }
    return out;
  },
  list_downloads: async () => Array.from(downloads.values()),
  cancel_download: async (a) => {
    cancelMockDownload(String(a.groupId));
  },
};

export default table;
