// Map pasted generation data (A1111 / Forge / ComfyUI / CivitAI names) onto
// stable-diffusion.cpp names and onto the active model family's dials.
//
// Pure. PRIVACY: plans contain the prompt; keep them in memory only.

import type { FamilyUi, FineTune, Shape } from "../types";
import type { ParsedGeneration } from "./parse";

// ------------------------------------------------------------------ sd.cpp names
// From stable-diffusion.cpp src/stable-diffusion.cpp `sample_method_to_str` /
// `scheduler_to_str` (pinned engine). Labels are shown in the Fine-tune drawer.

export const SD_SAMPLERS: { id: string; label: string }[] = [
  { id: "euler", label: "Euler" },
  { id: "euler_a", label: "Euler a" },
  { id: "heun", label: "Heun" },
  { id: "dpm2", label: "DPM2" },
  { id: "dpm++2s_a", label: "DPM++ 2S a" },
  { id: "dpm++2m", label: "DPM++ 2M" },
  { id: "dpm++2mv2", label: "DPM++ 2M v2" },
  { id: "dpm++2m_sde", label: "DPM++ 2M SDE" },
  { id: "dpm++2m_sde_bt", label: "DPM++ 2M SDE (Brownian)" },
  { id: "ipndm", label: "iPNDM" },
  { id: "ipndm_v", label: "iPNDM v" },
  { id: "lcm", label: "LCM" },
  { id: "ddim_trailing", label: "DDIM (trailing)" },
  { id: "tcd", label: "TCD" },
  { id: "res_multistep", label: "RES multistep" },
  { id: "res_2s", label: "RES 2S" },
  { id: "er_sde", label: "ER-SDE" },
  { id: "euler_cfg_pp", label: "Euler CFG++" },
  { id: "euler_a_cfg_pp", label: "Euler a CFG++" },
  { id: "euler_ge", label: "Euler (gradient estimation)" },
  { id: "lms", label: "LMS" },
];

export const SD_SCHEDULERS: { id: string; label: string }[] = [
  { id: "discrete", label: "Discrete (normal)" },
  { id: "karras", label: "Karras" },
  { id: "exponential", label: "Exponential" },
  { id: "ays", label: "Align Your Steps" },
  { id: "gits", label: "GITS" },
  { id: "sgm_uniform", label: "SGM uniform" },
  { id: "simple", label: "Simple" },
  { id: "smoothstep", label: "Smoothstep" },
  { id: "kl_optimal", label: "KL optimal" },
  { id: "lcm", label: "LCM" },
  { id: "beta", label: "Beta" },
];

export const samplerLabel = (id: string | null | undefined) =>
  id ? (SD_SAMPLERS.find((s) => s.id === id)?.label ?? id) : "Automatic";
export const schedulerLabel = (id: string | null | undefined) =>
  id ? (SD_SCHEDULERS.find((s) => s.id === id)?.label ?? id) : "Automatic";

// Normalised sampler key → [sd.cpp name | null, note when not exact]
const SAMPLER_TABLE: Record<string, [string | null, string | null]> = {
  euler: ["euler", null],
  eulera: ["euler_a", null],
  eulerancestral: ["euler_a", null],
  heun: ["heun", null],
  heunpp2: ["heun", "Heun++ isn't available; using Heun."],
  dpm2: ["dpm2", null],
  dpm2a: ["dpm++2s_a", "DPM2 a isn't available; using DPM++ 2S a (very similar)."],
  dpm2ancestral: ["dpm++2s_a", "DPM2 a isn't available; using DPM++ 2S a (very similar)."],
  dpmpp2sa: ["dpm++2s_a", null],
  dpmpp2sancestral: ["dpm++2s_a", null],
  dpmpp2m: ["dpm++2m", null],
  dpmpp2mv2: ["dpm++2mv2", null],
  dpmpp2msde: ["dpm++2m_sde", null],
  dpmpp2msdegpu: ["dpm++2m_sde", null],
  dpmpp2msdebt: ["dpm++2m_sde_bt", null],
  dpmpp2msdeheun: ["dpm++2m_sde", "DPM++ 2M SDE Heun isn't available; using DPM++ 2M SDE."],
  dpmpp3msde: ["dpm++2m_sde", "DPM++ 3M SDE isn't available; using DPM++ 2M SDE (closest)."],
  dpmpp3msdegpu: ["dpm++2m_sde", "DPM++ 3M SDE isn't available; using DPM++ 2M SDE (closest)."],
  dpmppsde: ["dpm++2m_sde", "DPM++ SDE isn't available; using DPM++ 2M SDE (closest)."],
  dpmppsdegpu: ["dpm++2m_sde", "DPM++ SDE isn't available; using DPM++ 2M SDE (closest)."],
  lms: ["lms", null],
  plms: ["ipndm", "PLMS isn't available; using iPNDM (closest)."],
  ipndm: ["ipndm", null],
  ipndmv: ["ipndm_v", null],
  lcm: ["lcm", null],
  ddim: ["ddim_trailing", "DDIM runs as DDIM (trailing) in Pinhole's engine."],
  ddimcfgpp: ["ddim_trailing", "DDIM CFG++ isn't available; using DDIM (trailing)."],
  ddimtrailing: ["ddim_trailing", null],
  tcd: ["tcd", null],
  resmultistep: ["res_multistep", null],
  res2s: ["res_2s", null],
  ersde: ["er_sde", null],
  eulercfgpp: ["euler_cfg_pp", null],
  euleracfgpp: ["euler_a_cfg_pp", null],
  eulerancestralcfgpp: ["euler_a_cfg_pp", null],
  gradientestimation: ["euler_ge", null],
  eulerge: ["euler_ge", null],
  unipc: [null, "UniPC isn't available in Pinhole's engine; using the model's default sampler."],
  unipcbh2: [null, "UniPC isn't available in Pinhole's engine; using the model's default sampler."],
  dpmfast: [null, "DPM fast isn't available; using the model's default sampler."],
  dpmadaptive: [null, "DPM adaptive isn't available; using the model's default sampler."],
  restart: [null, "Restart isn't available; using the model's default sampler."],
  deis: [null, "DEIS isn't available; using the model's default sampler."],
  ddpm: [null, "DDPM isn't available; using the model's default sampler."],
  seeds2: [null, "SEEDS isn't available; using the model's default sampler."],
  seeds3: [null, "SEEDS isn't available; using the model's default sampler."],
  sasolver: [null, "SA-Solver isn't available; using the model's default sampler."],
};

// Normalised scheduler key → [sd.cpp name | null, note]
const SCHEDULER_TABLE: Record<string, [string | null, string | null]> = {
  karras: ["karras", null],
  exponential: ["exponential", null],
  polyexponential: ["exponential", "Polyexponential isn't available; using Exponential."],
  sgmuniform: ["sgm_uniform", null],
  simple: ["simple", null],
  normal: ["discrete", null],
  discrete: ["discrete", null],
  uniform: ["discrete", "Uniform schedule runs as Discrete."],
  ddimuniform: ["discrete", "DDIM uniform schedule runs as Discrete."],
  alignyoursteps: ["ays", null],
  ays: ["ays", null],
  alignyourstepsgits: ["gits", null],
  gits: ["gits", null],
  kloptimal: ["kl_optimal", null],
  beta: ["beta", null],
  lcm: ["lcm", null],
  smoothstep: ["smoothstep", null],
  linearquadratic: [null, "Linear-quadratic schedule isn't available; using the model's default."],
  turbo: [null, "Turbo schedule isn't available; using the model's default."],
  automatic: [null, null],
  auto: [null, null],
  default: [null, null],
};

// Scheduler words that A1111 used to append to sampler names ("DPM++ 2M Karras").
const SCHEDULER_SUFFIXES = [
  "align your steps",
  "polyexponential",
  "sgm uniform",
  "kl optimal",
  "exponential",
  "karras",
  "simple",
  "normal",
  "beta",
  "uniform",
  "automatic",
];

const normKey = (s: string) =>
  s
    .toLowerCase()
    .replace(/\+\+/g, "pp")
    .replace(/\+/g, "p")
    .replace(/[^a-z0-9]/g, "");

export interface SamplerMapping {
  sampler: string | null;
  scheduler: string | null;
  /** Human notes about approximations / unsupported names. */
  notes: string[];
  /** Both names were recognised and mapped exactly. */
  exact: boolean;
}

/** "DPM++ 2M Karras" → { sampler: "dpm++2m", scheduler: "karras" }. */
export function mapSampler(rawSampler: string | null | undefined, rawScheduler: string | null | undefined): SamplerMapping {
  const notes: string[] = [];
  let exact = true;
  let sampler: string | null = null;
  let scheduler: string | null = null;
  let suffixScheduler: string | null = null;

  let s = (rawSampler ?? "").trim().replace(/^k_(?=[a-z])/i, ""); // old "k_euler_a" style
  if (/^(undefined|none|null|n\/a|-)$/i.test(s)) s = "";
  if (s) {
    // Split an A1111 scheduler suffix ("DPM++ 2M SDE Karras") or a Comfy one ("dpmpp_2m_karras").
    const spaced = s.toLowerCase().replace(/_/g, " ").replace(/\s+/g, " ").trim();
    for (const suf of SCHEDULER_SUFFIXES) {
      if (spaced.endsWith(` ${suf}`) && spaced.length > suf.length + 1) {
        const base = spaced.slice(0, -suf.length - 1);
        if (SAMPLER_TABLE[normKey(base)]) {
          suffixScheduler = suf;
          s = base;
        }
        break;
      }
    }
    const hit = SAMPLER_TABLE[normKey(s)];
    if (hit) {
      sampler = hit[0];
      if (hit[1]) {
        notes.push(hit[1]);
        exact = false;
      }
    } else {
      notes.push(`Sampler "${rawSampler!.trim()}" isn't known to Pinhole; using the model's default sampler.`);
      exact = false;
    }
  }

  // An explicit "Schedule type"/"Scheduler" wins over the suffix, unless it says Automatic.
  let sch = (rawScheduler ?? "").trim();
  const explicit = sch ? SCHEDULER_TABLE[normKey(sch)] : undefined;
  const explicitIsAuto = !!explicit && explicit[0] === null && explicit[1] === null;
  if (!sch || explicitIsAuto) sch = suffixScheduler ?? sch;
  if (sch) {
    const hit = SCHEDULER_TABLE[normKey(sch)];
    if (hit) {
      scheduler = hit[0];
      if (hit[1]) {
        notes.push(hit[1]);
        exact = false;
      }
    } else {
      notes.push(`Schedule "${sch}" isn't known to Pinhole; using the model's default.`);
      exact = false;
    }
  }
  return { sampler, scheduler, notes, exact };
}

// ------------------------------------------------------------------ dial math (shared with the UI)

/** Dial position (0…1) of the family's default "Stick to prompt" value. */
export function defaultStickPosition(ui: FamilyUi | null | undefined): number {
  if (!ui) return 0.5;
  const [lo, hi] = ui.stickRange;
  const d = ui.stickDefault;
  if (!(hi > lo)) return 0.5;
  if (d >= lo && d <= hi) return clamp01((d - lo) / (hi - lo));
  // Some backends may already send a 0…1 position.
  if (d >= 0 && d <= 1) return d;
  return 0.5;
}

/** Concrete CFG/guidance value for a dial position. */
export function stickValue(ui: FamilyUi, position: number): number {
  const [lo, hi] = ui.stickRange;
  return round2(lo + clamp01(position) * (hi - lo));
}

/** Dial position for a concrete value (clamped). */
export function stickPositionFor(ui: FamilyUi, value: number): number {
  const [lo, hi] = ui.stickRange;
  if (!(hi > lo)) return 0.5;
  return clamp01((value - lo) / (hi - lo));
}

/** Pick the shape chip matching a size exactly, else the closest aspect ratio. */
export function shapeFor(ui: FamilyUi | null, w: number, h: number): { shape: Shape; exact: boolean } {
  const shapes = ui?.shapes ?? {
    square: [1024, 1024],
    portrait: [832, 1216],
    landscape: [1216, 832],
    wide: [1344, 768],
  };
  let best: Shape = "square";
  let bestDiff = Infinity;
  for (const [name, [sw, sh]] of Object.entries(shapes)) {
    if (sw === w && sh === h) return { shape: name as Shape, exact: true };
    const diff = Math.abs(Math.log(w / h) - Math.log(sw / sh));
    if (diff < bestDiff) {
      bestDiff = diff;
      best = name as Shape;
    }
  }
  return { shape: best, exact: false };
}

export const SHAPE_LABEL: Record<Shape, string> = { square: "Square", portrait: "Portrait", landscape: "Landscape", wide: "Wide" };

const clamp01 = (n: number) => Math.min(1, Math.max(0, n));
const round2 = (n: number) => Math.round(n * 100) / 100;

// ------------------------------------------------------------------ apply plan

export interface PasteSkip {
  what: string;
  why: string;
}

export interface PastePlan {
  /** prompt-bearing */
  prompt: string;
  /** Complete Fine-tune values to use (everything else back to the registry default). prompt-bearing (negative). */
  fineTune: FineTune;
  shape: Shape | null;
  /** Dial position 0…1 for "Stick to prompt", or null to keep the family default. */
  stick: number | null;
  keepLook: boolean;
  applied: string[];
  skipped: PasteSkip[];
  /** Setting names we saw but don't use (names only — values may contain text). */
  ignoredKeys: string[];
}

// Extra keys that are pure bookkeeping — not worth mentioning.
const QUIET_KEYS = new Set([
  "created date",
  "civitai metadata",
  "version",
  "workflow",
  "draft",
  "fluxmode",
  "flux mode",
  "baseModel".toLowerCase(),
  "remixofid",
  "hires resize",
  "ensd",
  "eta",
  "rng",
  "vae",
  "schedule max sigma",
  "schedule min sigma",
  "schedule rho",
  "sgm noise multiplier",
  "emphasis",
  "downcast alphas_cumprod",
]);

/**
 * Turn parsed data into concrete changes for the model family `ui` (the model
 * that will be used after resource resolution). `ui` may be null when no model
 * is installed: values are then applied without family checks.
 */
export function planPaste(p: ParsedGeneration, ui: FamilyUi | null): PastePlan {
  const ft: FineTune = {};
  const applied: string[] = [];
  const skipped: PasteSkip[] = [];
  let shape: Shape | null = null;
  let stick: number | null = null;
  let keepLook = false;
  const family = ui?.label ?? "This model";

  if (p.prompt) applied.push("Prompt");

  // Negative prompt
  if (p.negative) {
    if (!ui || ui.usesNegativePrompt) {
      ft.negativePrompt = p.negative;
      applied.push("Negative prompt");
    } else {
      skipped.push({ what: "Negative prompt", why: `${family} doesn't use one` });
    }
  }

  // Steps
  if (p.steps != null) {
    ft.steps = p.steps;
    applied.push(`${p.steps} steps`);
  }

  // Sampler / scheduler
  if (p.sampler || p.scheduler) {
    const m = mapSampler(p.sampler, p.scheduler);
    if (m.sampler) ft.sampler = m.sampler;
    if (m.scheduler) ft.scheduler = m.scheduler;
    const shown = [m.sampler ? samplerLabel(m.sampler) : null, m.scheduler ? schedulerLabel(m.scheduler) : null].filter(Boolean).join(" · ");
    const src = [p.sampler, p.scheduler && !(p.sampler ?? "").toLowerCase().includes(p.scheduler.toLowerCase()) ? p.scheduler : null]
      .filter(Boolean)
      .join(", ");
    if (shown) applied.push(m.exact ? `Sampler ${shown}` : `Sampler ${shown} (from ${src})`);
    for (const n of m.notes) skipped.push({ what: "Sampler", why: n });
  }

  // CFG / guidance → the family's "Stick to prompt" dial
  const cfg = p.cfg;
  let guidance = p.guidance;
  if (!ui) {
    if (cfg != null) {
      ft.cfg = cfg;
      applied.push(`CFG ${cfg}`);
    }
    if (guidance != null) {
      ft.guidance = guidance;
      applied.push(`Guidance ${guidance}`);
    }
  } else if (ui.stickMapsTo === "guidance") {
    // Flux-style: real CFG is fixed (usually 1); the dial is distilled guidance.
    if (guidance == null && cfg != null && cfg > 1.01) {
      // CivitAI's Flux generator writes guidance as "CFG scale".
      guidance = cfg;
      skipped.push({ what: `CFG ${cfg}`, why: `${family} has a fixed CFG — used it as guidance instead` });
    } else if (cfg != null && Math.abs(cfg - ui.defaultCfg) > 0.01) {
      skipped.push({ what: `CFG ${cfg}`, why: `${family} uses a fixed CFG of ${ui.defaultCfg}` });
    }
    if (guidance != null) {
      ft.guidance = guidance;
      if (ui.showStick) stick = stickPositionFor(ui, guidance);
      applied.push(`Stick to prompt: guidance ${guidance}`);
    }
  } else {
    if (cfg != null) {
      if (ui.showStick) {
        ft.cfg = cfg;
        stick = stickPositionFor(ui, cfg);
        const [lo, hi] = ui.stickRange;
        applied.push(cfg < lo || cfg > hi ? `Stick to prompt: CFG ${cfg} (outside the usual ${lo}–${hi})` : `Stick to prompt: CFG ${cfg}`);
      } else if (Math.abs(cfg - ui.defaultCfg) > 0.01) {
        skipped.push({ what: `CFG ${cfg}`, why: `${family} uses a fixed CFG of ${ui.defaultCfg}` });
      }
    }
    if (guidance != null) skipped.push({ what: `Guidance ${guidance}`, why: `${family} doesn't use guidance` });
  }

  // Seed → Keep this look
  if (p.seed != null) {
    ft.seed = p.seed;
    keepLook = true;
    applied.push(`Seed ${p.seed} (Keep this look is on)`);
  }

  // Size → shape chip or custom size
  if (p.width != null && p.height != null) {
    const w = roundTo(p.width, 8);
    const h = roundTo(p.height, 8);
    const m = shapeFor(ui, w, h);
    shape = m.shape;
    if (m.exact) {
      applied.push(`Size ${w}×${h} (${SHAPE_LABEL[m.shape]})`);
    } else {
      ft.width = w;
      ft.height = h;
      applied.push(`Size ${w}×${h} (custom)`);
    }
  }

  // Clip skip
  if (p.clipSkip != null) {
    if (!ui || ui.defaultClipSkip != null) {
      ft.clipSkip = p.clipSkip;
      applied.push(`Clip skip ${p.clipSkip}`);
    } else if (p.clipSkip > 1) {
      skipped.push({ what: `Clip skip ${p.clipSkip}`, why: `${family} doesn't use clip skip` });
    }
  }

  // Hires fix
  if (p.hires) {
    ft.hires = true;
    if (p.hires.scale != null) ft.hiresScale = p.hires.scale;
    if (p.denoise != null) ft.hiresDenoise = p.denoise;
    applied.push(
      `Hires fix${p.hires.scale != null ? ` ×${p.hires.scale}` : ""}${p.denoise != null ? `, strength ${p.denoise}` : ""}`,
    );
    if (p.hires.steps) skipped.push({ what: `Hires steps ${p.hires.steps}`, why: "Pinhole picks hires steps automatically" });
    if (p.hires.upscaler && !/^(none|latent.*)$/i.test(p.hires.upscaler)) {
      skipped.push({ what: `Upscaler "${p.hires.upscaler}"`, why: "Pinhole uses its own hires upscaler" });
    }
  } else if (p.denoise != null && p.denoise < 1) {
    skipped.push({ what: `Denoising strength ${p.denoise}`, why: "only used for hires fix or Restyle" });
  }

  // Auto prompt prefix: don't add Pony score tags twice.
  if (ui?.autoPromptPrefix && p.prompt && promptHasPrefix(p.prompt, ui.autoPromptPrefix)) {
    ft.autoPromptPrefix = false;
    applied.push("Automatic prompt prefix off (the pasted prompt already has it)");
  }

  for (const h of p.hypernetworks) skipped.push({ what: `Hypernetwork "${h}"`, why: "not supported" });

  const ignoredKeys: string[] = [];
  let adetailer = false;
  for (const k of Object.keys(p.extra)) {
    const kl = k.toLowerCase();
    if (kl.startsWith("adetailer")) {
      adetailer = true;
      continue;
    }
    if (QUIET_KEYS.has(kl) || /^module \d+$/.test(kl)) continue;
    ignoredKeys.push(k);
  }
  if (adetailer) skipped.push({ what: "ADetailer (face fix)", why: "not available in Pinhole yet" });

  return { prompt: p.prompt, fineTune: ft, shape, stick, keepLook, applied, skipped, ignoredKeys };
}

function roundTo(n: number, m: number): number {
  return Math.max(m, Math.round(n / m) * m);
}

/** Does the prompt already start with (all tags of) the family's automatic prefix? */
export function promptHasPrefix(prompt: string, prefix: string): boolean {
  const tags = prefix
    .split(",")
    .map((t) => t.trim().toLowerCase())
    .filter(Boolean);
  if (!tags.length) return false;
  const head = prompt
    .toLowerCase()
    .split(/[,\n]/)
    .map((t) => t.trim())
    .slice(0, tags.length + 8);
  return tags.every((t) => head.includes(t));
}
