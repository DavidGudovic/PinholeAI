// Mock handlers for the library area (styles + presets). See ./index.ts.
// RAM only; reset on reload. Built-ins mirror config/styles and config/presets.
import type { MockTable } from "./index";
import type { CoreError, Preset, Style } from "../types";

const err = (code: string, message: string): CoreError => ({ code, message, details: null });
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const BUILTIN_STYLES: Style[] = [
  {
    id: "builtin:film-photo",
    name: "Film photo",
    positive: "35mm film photograph, soft natural light, shallow depth of field, fine grain, natural skin texture",
    negative: "cartoon, illustration, plastic skin, oversaturated",
    families: [],
    thumbnail: null,
    builtin: true,
  },
  {
    id: "builtin:product-white",
    name: "Product shot on white",
    positive: "professional studio product photography, seamless white background, softbox lighting, sharp focus, subtle shadow",
    negative: "clutter, busy background, text, watermark",
    families: [],
    thumbnail: null,
    builtin: true,
  },
  {
    id: "builtin:anime-cel",
    name: "Anime cel shading",
    positive: "anime style, clean line art, cel shading, vibrant colors",
    negative: "photorealistic, 3d render",
    families: ["sdxl_illustrious", "sdxl_pony", "sdxl"],
    thumbnail: null,
    builtin: true,
  },
  {
    id: "builtin:watercolor",
    name: "Watercolor",
    positive: "watercolor painting, soft washes, visible paper texture, loose brush strokes",
    negative: "photo, 3d render, harsh outlines",
    families: [],
    thumbnail: null,
    builtin: true,
  },
];

const BUILTIN_PRESETS: Preset[] = [
  {
    id: "builtin:anime-illustration",
    name: "Anime illustration",
    family: "sdxl_illustrious",
    modelId: null,
    civitaiVersionId: null,
    styleId: "builtin:anime-cel",
    shape: "portrait",
    quality: "balanced",
    stick: 0.5,
    count: 2,
    fineTune: {},
    loras: [],
    builtin: true,
  },
  {
    id: "builtin:photo-portrait",
    name: "Photo portrait",
    family: "z_image_turbo",
    modelId: null,
    civitaiVersionId: null,
    styleId: "builtin:film-photo",
    shape: "portrait",
    quality: "balanced",
    stick: null,
    count: 2,
    fineTune: {},
    loras: [],
    builtin: true,
  },
  {
    id: "builtin:product-white",
    name: "Product shot on white",
    family: "z_image_turbo",
    modelId: null,
    civitaiVersionId: null,
    styleId: "builtin:product-white",
    shape: "square",
    quality: "best",
    stick: null,
    count: 4,
    fineTune: {},
    loras: [],
    builtin: true,
  },
];

let userStyles: Style[] = [];
let userPresets: Preset[] = [];

const slug = (s: string) =>
  s
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 40) || "item";

function uniqueId(base: string, taken: string[]): string {
  let id = base;
  for (let i = 2; taken.includes(id); i++) id = `${base}-${i}`;
  return id;
}

/** Rust order: built-ins by name, then the user's by name. */
const byName = <T extends { name: string; id: string }>(list: T[]) =>
  list.slice().sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }) || a.id.localeCompare(b.id));

export function allStyles(): Style[] {
  return [...byName(BUILTIN_STYLES), ...byName(userStyles)];
}

export function styleById(id: string | null | undefined): Style | null {
  return id ? (allStyles().find((s) => s.id === id) ?? null) : null;
}

const table: MockTable = {
  list_styles: async () => {
    await sleep(60);
    return allStyles();
  },
  save_style: async (a) => {
    await sleep(120);
    const st = a.style as Style;
    if (!st?.name?.trim() || !st.positive?.trim()) throw err("invalid", "Give the style a name and describe the look.");
    if (st.builtin || BUILTIN_STYLES.some((b) => b.id === st.id)) throw err("invalid", "Built-in styles can't be changed. Duplicate it to make your own.");
    const existing = userStyles.find((s) => s.id === st.id);
    // Rust: a non-empty id must name an existing user style.
    if (st.id?.trim() && !existing) throw err("not_found", "Style was not found. It may have been deleted.");
    const saved: Style = {
      ...st,
      id: existing ? existing.id : uniqueId(slug(st.name), allStyles().map((s) => s.id)),
      name: st.name.trim(),
      builtin: false,
    };
    userStyles = existing ? userStyles.map((s) => (s.id === saved.id ? saved : s)) : [...userStyles, saved];
    return saved;
  },
  delete_style: async (a) => {
    await sleep(80);
    const id = String(a.id);
    if (BUILTIN_STYLES.some((b) => b.id === id)) throw err("invalid", "Built-in styles can't be deleted.");
    userStyles = userStyles.filter((s) => s.id !== id);
  },
  list_presets: async () => {
    await sleep(60);
    return [...byName(BUILTIN_PRESETS), ...byName(userPresets)];
  },
  save_preset: async (a) => {
    await sleep(120);
    const p = a.preset as Preset;
    if (!p?.name?.trim()) throw err("invalid", "Give the preset a name.");
    if (BUILTIN_PRESETS.some((b) => b.id === p.id)) throw err("invalid", "Built-in presets can't be changed.");
    // Defensive, like the Rust side: presets never hold a negative prompt.
    const { negativePrompt: _n, hiresScale: _s, hiresDenoise: _d, ...fineTune } = (p.fineTune ?? {}) as Record<string, unknown>;
    void _n;
    void _s;
    void _d;
    const existing = userPresets.find((x) => x.id === p.id);
    if (p.id?.trim() && !existing) throw err("not_found", "Preset was not found. It may have been deleted.");
    const saved: Preset = {
      ...p,
      fineTune,
      id: existing ? existing.id : uniqueId(slug(p.name), [...userPresets, ...BUILTIN_PRESETS].map((x) => x.id)),
      name: p.name.trim(),
      builtin: false,
    };
    userPresets = existing ? userPresets.map((x) => (x.id === saved.id ? saved : x)) : [...userPresets, saved];
    return saved;
  },
  delete_preset: async (a) => {
    await sleep(80);
    const id = String(a.id);
    if (BUILTIN_PRESETS.some((b) => b.id === id)) throw err("invalid", "Built-in presets can't be deleted.");
    userPresets = userPresets.filter((x) => x.id !== id);
  },
};

export default table;
