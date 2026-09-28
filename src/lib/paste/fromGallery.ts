// Model details page → Create: turn a CivitAI preview image's generation data
// (the `meta` object) into the same text "Copy generation data" produces, so
// it goes through the one Paste-from-CivitAI path (parse → resolve → apply).
// PRIVACY: in memory only; never log or store the result.

/** The model whose page the image came from (so Create can select or offer it). */
export interface GallerySource {
  type: string;
  versionId: number;
  modelName: string;
  versionName: string;
}

type Meta = Record<string, unknown>;

/** Setting keys in A1111 order, each with the meta keys it can come from. */
const SETTINGS: [string, string[]][] = [
  ["Steps", ["steps"]],
  ["Sampler", ["sampler"]],
  ["Schedule type", ["scheduler", "Schedule type"]],
  ["CFG scale", ["cfgScale"]],
  ["Distilled CFG Scale", ["guidance", "Distilled CFG Scale"]],
  ["Seed", ["seed"]],
  ["Size", ["Size"]],
  ["Clip skip", ["clipSkip", "Clip skip"]],
  ["Model", ["Model"]],
  ["Model hash", ["Model hash"]],
  ["VAE", ["VAE"]],
  ["VAE hash", ["VAE hash"]],
  ["Lora hashes", ["Lora hashes"]],
  ["TI hashes", ["TI hashes"]],
  ["Denoising strength", ["Denoising strength"]],
  ["Hires upscale", ["Hires upscale"]],
  ["Hires steps", ["Hires steps"]],
  ["Hires upscaler", ["Hires upscaler"]],
];

function scalar(v: unknown): string | null {
  if (typeof v === "number" && Number.isFinite(v)) return String(v);
  if (typeof v === "string" && v.trim()) return v.trim();
  return null;
}

/** Quote values the settings tokenizer would otherwise split. */
function settingValue(v: string): string {
  return /[,"\n]/.test(v) || /^[[{]/.test(v) ? JSON.stringify(v) : v;
}

const isObj = (v: unknown): v is Meta => !!v && typeof v === "object" && !Array.isArray(v);
const normType = (t: unknown) => String(t ?? "").toLowerCase().replace(/[^a-z]/g, "");

/**
 * Resources for "Civitai resources": CivitAI's own list, A1111 `resources`
 * (name + hash), and the model this page is about (so it gets selected, or
 * offered for install, even when the image data doesn't name it).
 */
function resources(meta: Meta, source: GallerySource | null): Meta[] {
  const out: Meta[] = [];
  for (const r of Array.isArray(meta.civitaiResources) ? meta.civitaiResources : []) if (isObj(r)) out.push(r);
  for (const r of Array.isArray(meta.resources) ? meta.resources : []) {
    if (!isObj(r)) continue;
    const t = normType(r.type);
    if (t !== "model" && t !== "checkpoint" && t !== "lora" && t !== "locon" && t !== "lycoris") continue;
    out.push({ type: t === "model" ? "checkpoint" : t, modelName: r.name, hash: r.hash, weight: r.weight });
  }
  if (source && source.versionId > 0) {
    const t = normType(source.type);
    const kind = t === "checkpoint" ? "checkpoint" : t === "lora" || t === "locon" || t === "dora" ? "lora" : null;
    const already = out.some((r) => Number(r.modelVersionId ?? r.versionId) === source.versionId);
    // A LoRA named only by file name or hash (resources, "Lora hashes", a <lora:…> tag) can't be
    // matched to this page's version id; adding ours too would apply the same LoRA twice. Images on
    // a LoRA's page almost always use it, so trust the image's own list when it has one.
    const namesLora =
      kind === "lora" &&
      (out.some((r) => normType(r.type) === "lora" || normType(r.type) === "locon" || normType(r.type) === "lycoris") ||
        scalar(meta["Lora hashes"]) != null ||
        /<(lora|lyco):/i.test(String(meta.prompt ?? "")));
    if (kind && !already && !namesLora) {
      // This page's checkpoint wins over one named only by hash; a LoRA joins the others.
      const mine: Meta = { type: kind, modelVersionId: source.versionId, modelName: source.modelName, modelVersionName: source.versionName };
      if (kind === "lora") mine.weight = 1;
      const rest = kind === "checkpoint" ? out.filter((r) => normType(r.type) !== "checkpoint" || r.modelVersionId != null) : out;
      return [mine, ...rest];
    }
  }
  return out;
}

/** A1111-style generation text for a gallery image, or null when there is nothing to apply. */
export function galleryGenerationText(meta: Meta | null | undefined, source: GallerySource | null): string | null {
  if (!isObj(meta)) return null;
  const prompt = scalar(meta.prompt) ?? "";
  const negative = scalar(meta.negativePrompt);
  const pairs: string[] = [];
  for (const [label, keys] of SETTINGS) {
    const v = keys.map((k) => scalar(meta[k])).find((x) => x != null);
    if (v != null) pairs.push(`${label}: ${settingValue(v)}`);
  }
  if (isObj(meta.hashes)) pairs.push(`Hashes: ${JSON.stringify(meta.hashes)}`);
  const res = resources(meta, source);
  if (res.length) pairs.push(`Civitai resources: ${JSON.stringify(res)}`);
  if (!prompt && !pairs.some((p) => p.startsWith("Steps:"))) return null;
  // The parser needs at least two settings to find the settings line.
  if (pairs.length < 2) return prompt || null;
  return [prompt, negative != null ? `Negative prompt: ${negative}` : null, pairs.join(", ")].filter((l): l is string => l != null && l !== "").join("\n");
}

/** Short settings summary for the image viewer ("30 steps · CFG 7 · 832×1216"). */
export function gallerySettingsSummary(meta: Meta | null | undefined): string[] {
  if (!isObj(meta)) return [];
  const out: string[] = [];
  const steps = scalar(meta.steps);
  if (steps) out.push(`${steps} steps`);
  const cfg = scalar(meta.cfgScale);
  if (cfg) out.push(`CFG ${cfg}`);
  const sampler = scalar(meta.sampler);
  if (sampler) out.push(sampler);
  const size = scalar(meta.Size);
  if (size) out.push(size.replace("x", "×"));
  const seed = scalar(meta.seed);
  if (seed) out.push(`Seed ${seed}`);
  return out;
}
