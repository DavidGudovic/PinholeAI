// Parser for "generation data" text copied from CivitAI ("Copy generation data"),
// AUTOMATIC1111 / Forge infotext and similar tools.
//
// Pure and synchronous. PRIVACY: the input is prompt-bearing. This module never
// logs, stores or sends it anywhere; callers keep the result in memory only.
//
// Shape of the input (every part optional except the prompt):
//
//   <prompt lines, may contain <lora:name:0.8> tags>
//   Negative prompt: <negative lines>
//   Steps: 30, Sampler: DPM++ 2M Karras, CFG scale: 7, Seed: 1, Size: 832x1216,
//   Lora hashes: "a: 0123abcd, b: 4567ef", Civitai resources: [{...}], ...

import type { PastedResource } from "../types";

export interface HiresInfo {
  scale: number | null;
  steps: number | null;
  upscaler: string | null;
}

export interface LoraTag {
  name: string;
  weight: number;
}

export interface ParsedGeneration {
  /** Prompt with <lora:…> / <lyco:…> / <hypernet:…> tags removed. */
  prompt: string;
  negative: string | null;
  steps: number | null;
  cfg: number | null;
  /** Flux-style distilled guidance ("Guidance", "Distilled CFG Scale"). */
  guidance: number | null;
  /** Raw sampler name as written (map with ./map.ts). */
  sampler: string | null;
  /** Raw scheduler name ("Schedule type" / "Scheduler"). */
  scheduler: string | null;
  seed: number | null;
  width: number | null;
  height: number | null;
  clipSkip: number | null;
  /** "Denoising strength" (hires fix or img2img). */
  denoise: number | null;
  hires: HiresInfo | null;
  modelName: string | null;
  modelHash: string | null;
  /** <lora:name:weight> tags found in the prompt (and removed from it). */
  loraTags: LoraTag[];
  /** Hypernetwork tags removed from the prompt (not supported by the engine). */
  hypernetworks: string[];
  /** Checkpoint, LoRAs, embeddings, VAE — ids/hashes only, for resolve_civitai_resources. */
  resources: PastedResource[];
  /** Every setting we read but don't map ("RNG", "VAE", "ADetailer model", …). */
  extra: Record<string, string>;
  /** A settings line (Steps:, Sampler:, …) was found. */
  hasSettings: boolean;
}

// Keys that mark a settings line. Lowercase.
const SETTING_KEYS = new Set([
  "steps",
  "sampler",
  "schedule type",
  "scheduler",
  "cfg scale",
  "cfg",
  "seed",
  "size",
  "model hash",
  "model",
  "clip skip",
  "denoising strength",
  "denoise",
  "guidance",
  "distilled cfg scale",
  "hires upscale",
  "hires steps",
  "hires upscaler",
  "hires resize",
  "lora hashes",
  "ti hashes",
  "hashes",
  "civitai resources",
  "civitai metadata",
  "created date",
  "version",
  "vae",
  "vae hash",
  "rng",
  "eta",
  "ensd",
  "workflow",
]);

const MAX_INPUT = 200_000; // generous; CivitAI "workflow" blobs can be large

/** Cheap check used to offer "Apply these settings?" when text is pasted into the prompt box. */
export function looksLikeGenerationData(text: string): boolean {
  if (!text || text.length > MAX_INPUT) return false;
  const lines = normalizeNewlines(text).split("\n");
  for (let i = lines.length - 1; i >= 0; i--) {
    if (countSettingKeys(lines[i]) >= 3) return true;
  }
  // One-setting-per-line layout.
  let perLine = 0;
  for (const l of lines) if (startsWithSettingKey(l)) perLine++;
  if (perLine >= 3 && /(^|\n)\s*steps\s*:/i.test(text)) return true;
  return /(^|\n)\s*negative prompt\s*:/i.test(text) && lines.some((l) => countSettingKeys(l) >= 2);
}

/** Parse generation data. Returns null for empty input. Never throws. */
export function parseGenerationData(input: string): ParsedGeneration | null {
  if (typeof input !== "string") return null;
  let text = normalizeNewlines(input.length > MAX_INPUT ? input.slice(0, MAX_INPUT) : input).replace(/^﻿/, "");
  text = text.trim();
  if (!text) return null;

  const lines = text.split("\n");
  const settingsStart = findSettingsStart(lines);
  const head = settingsStart >= 0 ? lines.slice(0, settingsStart) : lines;
  const settingsText = settingsStart >= 0 ? lines.slice(settingsStart).join("\n") : "";

  // ---- prompt / negative
  let promptLines = head;
  let negative: string | null = null;
  const negIdx = head.findIndex((l) => /^\s*negative prompt\s*:/i.test(l));
  if (negIdx >= 0) {
    promptLines = head.slice(0, negIdx);
    const first = head[negIdx].replace(/^\s*negative prompt\s*:\s*/i, "");
    negative = [first, ...head.slice(negIdx + 1)].join("\n").trim();
  }
  let prompt = promptLines.join("\n").trim();

  // ---- tags inside the prompt
  const loraTags: LoraTag[] = [];
  const hypernetworks: string[] = [];
  prompt = stripExtraNetworkTags(prompt, loraTags, hypernetworks);
  if (negative != null) {
    // LoRAs in the negative prompt are rare but possible; they still get removed.
    negative = stripExtraNetworkTags(negative, [], []);
    if (!negative) negative = null;
  }

  const out: ParsedGeneration = {
    prompt,
    negative,
    steps: null,
    cfg: null,
    guidance: null,
    sampler: null,
    scheduler: null,
    seed: null,
    width: null,
    height: null,
    clipSkip: null,
    denoise: null,
    hires: null,
    modelName: null,
    modelHash: null,
    loraTags,
    hypernetworks,
    resources: [],
    extra: {},
    hasSettings: settingsStart >= 0,
  };

  // ---- settings
  const pairs = settingsText ? tokenizeSettings(settingsText) : [];
  let civitaiResources: unknown = null;
  let hashesJson: unknown = null;
  const loraHashes = new Map<string, string>();
  const tiHashes = new Map<string, string>();
  let vaeHash: string | null = null;
  let vaeName: string | null = null;
  const hires: HiresInfo = { scale: null, steps: null, upscaler: null };
  let hiresSeen = false;

  for (const [rawKey, value] of pairs) {
    const key = rawKey.toLowerCase().replace(/\s+/g, " ").trim();
    switch (key) {
      case "steps":
        out.steps = toInt(value, 1, 1000);
        break;
      case "sampler":
      case "sampler name":
        out.sampler = nonEmpty(value);
        break;
      case "schedule type":
      case "scheduler":
      case "schedule":
        out.scheduler = nonEmpty(value);
        break;
      case "cfg scale":
      case "cfg":
        out.cfg = toFloat(value, 0, 100);
        break;
      case "guidance":
      case "distilled cfg scale":
      case "flux guidance":
        out.guidance = toFloat(value, 0, 100);
        break;
      case "seed": {
        const s = toInt(value, 0, Number.MAX_SAFE_INTEGER);
        out.seed = s;
        break;
      }
      case "size": {
        const wh = parseSize(value);
        if (wh) [out.width, out.height] = wh;
        break;
      }
      case "clip skip":
        out.clipSkip = toInt(value, 1, 12);
        break;
      case "denoising strength":
      case "denoise":
        out.denoise = toFloat(value, 0, 1);
        break;
      case "hires upscale":
        hires.scale = toFloat(value, 1, 8);
        hiresSeen = hiresSeen || hires.scale != null;
        break;
      case "hires steps":
        hires.steps = toInt(value, 0, 1000);
        hiresSeen = true;
        break;
      case "hires upscaler":
        hires.upscaler = nonEmpty(value);
        hiresSeen = true;
        break;
      case "model":
        out.modelName = nonEmpty(value);
        break;
      case "model hash":
        out.modelHash = cleanHash(value);
        break;
      case "vae":
        vaeName = nonEmpty(value);
        out.extra[rawKey] = value;
        break;
      case "vae hash":
        vaeHash = cleanHash(value);
        break;
      case "lora hashes":
        for (const [n, h] of parseNameHashList(value)) loraHashes.set(n, h);
        break;
      case "ti hashes":
        for (const [n, h] of parseNameHashList(value)) tiHashes.set(n, h);
        break;
      case "hashes":
        hashesJson = safeJson(value);
        break;
      case "civitai resources":
        civitaiResources = safeJson(value);
        break;
      default:
        out.extra[rawKey] = value;
    }
  }

  if (hiresSeen) {
    // "Hires upscaler: None" or a scale of 1 alone means hires was off.
    const upscalerNone = hires.upscaler != null && /^none$/i.test(hires.upscaler);
    if (!(hires.scale == null && upscalerNone) && !(hires.scale != null && hires.scale <= 1 && hires.steps == null)) {
      out.hires = hires;
    }
  }

  out.resources = buildResources({
    civitaiResources,
    hashesJson,
    modelName: out.modelName,
    modelHash: out.modelHash,
    loraHashes,
    tiHashes,
    loraTags,
    vaeHash,
    vaeName,
  });
  return out;
}

// ------------------------------------------------------------------ helpers

function normalizeNewlines(s: string): string {
  return s.replace(/\r\n?/g, "\n");
}

const KEY_AT_START = /^\s*([A-Za-z][A-Za-z0-9 _\-/().+]{0,40}?)\s*:/;

function startsWithSettingKey(line: string): boolean {
  const m = KEY_AT_START.exec(line);
  return !!m && SETTING_KEYS.has(m[1].toLowerCase().replace(/\s+/g, " "));
}

/** Number of known "Key:" occurrences in a line (only at the start or after a comma). */
function countSettingKeys(line: string): number {
  let n = 0;
  const re = /(?:^|,)\s*([A-Za-z][A-Za-z0-9 _\-/().+]{0,40}?)\s*:/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(line))) {
    if (SETTING_KEYS.has(m[1].toLowerCase().replace(/\s+/g, " "))) n++;
  }
  return n;
}

/**
 * Index of the first settings line, or -1. A1111 writes all settings on the
 * last line; some tools write one per line at the end; a few wrap long lines.
 */
function findSettingsStart(lines: string[]): number {
  // 1. The last line that starts with a known key and has ≥ 2 known keys.
  for (let i = lines.length - 1; i >= 0; i--) {
    if (/^\s*negative prompt\s*:/i.test(lines[i])) break;
    if (startsWithSettingKey(lines[i]) && countSettingKeys(lines[i]) >= 2) {
      // Earlier lines that also look like settings belong to the same block
      // (one-key-per-line blocks or lines broken by a text editor).
      let start = i;
      while (start > 0 && startsWithSettingKey(lines[start - 1]) && !/^\s*negative prompt\s*:/i.test(lines[start - 1])) start--;
      return start;
    }
  }
  // 2. A trailing block of one-setting-per-line lines (≥ 2 of them).
  let start = lines.length;
  while (start > 0 && startsWithSettingKey(lines[start - 1])) start--;
  if (lines.length - start >= 2) return start;
  return -1;
}

/**
 * Split `Key: value, Key2: "quoted, value", Key3: [json, …]` into pairs.
 * Unquoted values end at a comma or newline. Fragments without a key are
 * appended to the previous value (e.g. `Model: foo, bar`).
 */
export function tokenizeSettings(s: string): [string, string][] {
  const out: [string, string][] = [];
  const n = s.length;
  let i = 0;
  const keyRe = /([A-Za-z][A-Za-z0-9 _\-/().+]{0,60}?)\s*:\s*/y;
  while (i < n) {
    // skip separators
    while (i < n && (s[i] === "," || s[i] === "\n" || s[i] === " " || s[i] === "\t")) i++;
    if (i >= n) break;
    keyRe.lastIndex = i;
    const km = keyRe.exec(s);
    if (!km) {
      // Junk fragment: read to the next separator and glue it to the previous value.
      const end = findUnquotedEnd(s, i);
      const frag = s.slice(i, end).trim();
      if (frag && out.length) out[out.length - 1][1] += `, ${frag}`;
      i = end;
      continue;
    }
    const key = km[1].trim();
    i = keyRe.lastIndex;
    let value: string;
    const c = s[i];
    if (c === '"') {
      const end = findStringEnd(s, i);
      const raw = s.slice(i, end);
      value = unquote(raw);
      i = end;
    } else if (c === "[" || c === "{") {
      const end = findBracketEnd(s, i);
      if (end > 0) {
        value = s.slice(i, end);
        i = end;
      } else {
        const e2 = findUnquotedEnd(s, i);
        value = s.slice(i, e2).trim();
        i = e2;
      }
    } else {
      const end = findUnquotedEnd(s, i);
      value = s.slice(i, end).trim();
      i = end;
    }
    out.push([key, value]);
  }
  return out;
}

function findUnquotedEnd(s: string, from: number): number {
  let i = from;
  while (i < s.length && s[i] !== "," && s[i] !== "\n") i++;
  return i;
}

/** `s[from]` is a quote. Returns the index just past the closing quote (or end). */
function findStringEnd(s: string, from: number): number {
  let i = from + 1;
  while (i < s.length) {
    if (s[i] === "\\") i += 2;
    else if (s[i] === '"') return i + 1;
    else i++;
  }
  return s.length;
}

/** `s[from]` is `[` or `{`. Returns the index past the matching bracket, or -1. */
function findBracketEnd(s: string, from: number): number {
  const stack: string[] = [];
  let i = from;
  while (i < s.length) {
    const c = s[i];
    if (c === '"') {
      i = findStringEnd(s, i);
      continue;
    }
    if (c === "[" || c === "{") stack.push(c === "[" ? "]" : "}");
    else if (c === "]" || c === "}") {
      if (stack.pop() !== c) return -1;
      if (!stack.length) return i + 1;
    }
    i++;
  }
  return -1;
}

function unquote(raw: string): string {
  if (raw.length >= 2 && raw.endsWith('"')) {
    try {
      const v = JSON.parse(raw);
      if (typeof v === "string") return v;
    } catch {
      /* fall through */
    }
    return raw.slice(1, -1).replace(/\\"/g, '"');
  }
  return raw.replace(/^"/, "");
}

function safeJson(v: string): unknown {
  try {
    return JSON.parse(v);
  } catch {
    return null;
  }
}

function nonEmpty(v: string): string | null {
  const t = v.trim();
  return t ? t : null;
}

function toFloat(v: string, min: number, max: number): number | null {
  const n = Number.parseFloat(v.trim());
  if (!Number.isFinite(n) || n < min || n > max) return null;
  return n;
}

function toInt(v: string, min: number, max: number): number | null {
  const t = v.trim();
  if (!/^-?\d+(\.0+)?$/.test(t)) return null;
  const n = Number.parseInt(t, 10);
  if (!Number.isSafeInteger(n) || n < min || n > max) return null;
  return n;
}

function parseSize(v: string): [number, number] | null {
  const m = /^\s*(\d{2,5})\s*[x×*]\s*(\d{2,5})\s*$/i.exec(v);
  if (!m) return null;
  const w = Number(m[1]);
  const h = Number(m[2]);
  if (w < 64 || h < 64 || w > 8192 || h > 8192) return null;
  return [w, h];
}

function cleanHash(v: string): string | null {
  const t = v.trim().replace(/^"|"$/g, "");
  return /^[0-9a-f]{8,64}$/i.test(t) ? t.toLowerCase() : null;
}

/** `name: hash, name2: hash2` (A1111 "Lora hashes" / "TI hashes"). */
function parseNameHashList(v: string): [string, string][] {
  const out: [string, string][] = [];
  for (const part of v.split(",")) {
    const idx = part.lastIndexOf(":");
    if (idx <= 0) continue;
    const name = part.slice(0, idx).trim();
    const hash = cleanHash(part.slice(idx + 1));
    if (name && hash) out.push([name, hash]);
  }
  return out;
}

/**
 * Remove `<lora:name:w>`, `<lyco:…>`, `<locon:…>` and `<hypernet:…>` tags,
 * collecting them, then tidy the leftover commas/spaces.
 */
function stripExtraNetworkTags(text: string, loras: LoraTag[], hypernets: string[]): string {
  const re = /<\s*(lora|lyco|locon|hypernet)\s*:\s*([^:>]+?)\s*(?::\s*([^:>]*?)\s*)?(?::[^>]*)?>/gi;
  let changed = false;
  const stripped = text.replace(re, (_m, kind: string, name: string, weight: string | undefined) => {
    changed = true;
    const k = kind.toLowerCase();
    if (k === "hypernet") {
      if (!hypernets.includes(name)) hypernets.push(name);
    } else {
      const w = weight != null ? Number.parseFloat(weight) : NaN;
      const existing = loras.find((l) => l.name === name);
      const weightVal = Number.isFinite(w) ? w : 1;
      if (existing) existing.weight = weightVal;
      else loras.push({ name, weight: weightVal });
    }
    return "";
  });
  if (!changed) return text;
  return stripped
    .split("\n")
    .map((line) =>
      line
        .replace(/[ \t]{2,}/g, " ")
        .replace(/\s*,(\s*,)+/g, ",")
        .replace(/^\s*,\s*/, "")
        .replace(/\s*,\s*$/, "")
        .replace(/\s+,/g, ",")
        .trim(),
    )
    .join("\n")
    .trim();
}

const normName = (s: string) =>
  s
    .toLowerCase()
    .replace(/\.(safetensors|ckpt|pt|pth|bin|gguf)$/, "")
    .replace(/[^a-z0-9]/g, "");

function normType(t: unknown): string {
  const s = String(t ?? "").toLowerCase().replace(/[^a-z]/g, "");
  if (s === "lycoris" || s === "locon" || s === "dora" || s === "lora") return "lora";
  if (s === "textualinversion" || s === "embedding" || s === "embed" || s === "ti") return "embed";
  if (s === "checkpoint" || s === "model") return "checkpoint";
  return s || "unknown";
}

function toVersionId(v: unknown): number | null {
  const n = typeof v === "number" ? v : typeof v === "string" && /^\d+$/.test(v.trim()) ? Number(v.trim()) : NaN;
  return Number.isSafeInteger(n) && n > 0 ? n : null;
}

function toWeight(v: unknown): number | null {
  const n = typeof v === "number" ? v : typeof v === "string" ? Number.parseFloat(v) : NaN;
  return Number.isFinite(n) ? n : null;
}

function str(v: unknown): string | null {
  return typeof v === "string" && v.trim() ? v.trim() : null;
}

function buildResources(src: {
  civitaiResources: unknown;
  hashesJson: unknown;
  modelName: string | null;
  modelHash: string | null;
  loraHashes: Map<string, string>;
  tiHashes: Map<string, string>;
  loraTags: LoraTag[];
  vaeHash: string | null;
  vaeName: string | null;
}): PastedResource[] {
  const res: PastedResource[] = [];

  // 1. CivitAI's own list (on-site generations): most reliable, has version ids.
  if (Array.isArray(src.civitaiResources)) {
    for (const item of src.civitaiResources) {
      if (!item || typeof item !== "object") continue;
      const o = item as Record<string, unknown>;
      const r: PastedResource = {
        type: normType(o.type),
        modelVersionId: toVersionId(o.modelVersionId ?? o.versionId ?? o.id),
        modelName: str(o.modelName) ?? str(o.name),
        modelVersionName: str(o.modelVersionName) ?? str(o.versionName),
        hash: typeof o.hash === "string" ? cleanHash(o.hash) : null,
        weight: toWeight(o.weight ?? o.strength),
      };
      if (r.modelVersionId == null && !r.hash && !r.modelName) continue;
      // de-duplicate by version id
      if (r.modelVersionId != null && res.some((x) => x.modelVersionId === r.modelVersionId)) continue;
      res.push(r);
    }
  }

  // 2. "Hashes" JSON (newer A1111): {"model": "…", "lora:name": "…", "embed:name": "…", "vae": "…"}
  const loraHashes = new Map(src.loraHashes);
  const tiHashes = new Map(src.tiHashes);
  let modelHash = src.modelHash;
  let vaeHash = src.vaeHash;
  if (src.hashesJson && typeof src.hashesJson === "object" && !Array.isArray(src.hashesJson)) {
    for (const [k, v] of Object.entries(src.hashesJson as Record<string, unknown>)) {
      if (typeof v !== "string") continue;
      const h = cleanHash(v);
      if (!h) continue;
      const kl = k.toLowerCase();
      if (kl === "model") modelHash = modelHash ?? h;
      else if (kl === "vae") vaeHash = vaeHash ?? h;
      else if (kl.startsWith("lora:") || kl.startsWith("lyco:")) {
        const name = k.slice(5);
        if (!loraHashes.has(name)) loraHashes.set(name, h);
      } else if (kl.startsWith("embed:")) {
        const name = k.slice(6);
        if (!tiHashes.has(name)) tiHashes.set(name, h);
      }
    }
  }

  // 3. Checkpoint from "Model hash" / "Model".
  const ckpt = res.find((r) => r.type === "checkpoint");
  if (ckpt) {
    if (!ckpt.hash && modelHash) ckpt.hash = modelHash;
  } else if (modelHash || src.modelName) {
    res.unshift({ type: "checkpoint", modelVersionId: null, modelName: src.modelName, modelVersionName: null, hash: modelHash, weight: null });
  }

  // 4. LoRAs from tags + hashes, merged by name with CivitAI's list.
  const civitaiLoras = res.filter((r) => r.type === "lora");
  const names: string[] = [];
  for (const t of src.loraTags) if (!names.includes(t.name)) names.push(t.name);
  for (const n of loraHashes.keys()) if (!names.includes(n)) names.push(n);
  for (const name of names) {
    const hash = loraHashes.get(name) ?? null;
    const tag = src.loraTags.find((t) => t.name === name);
    const key = normName(name);
    const match = civitaiLoras.find(
      (r) => (r.modelName && normName(r.modelName) === key) || (r.modelVersionName && normName(r.modelVersionName) === key) || (hash && r.hash === hash),
    );
    if (match) {
      if (!match.hash && hash) match.hash = hash;
      if (match.weight == null && tag) match.weight = tag.weight;
      continue;
    }
    // When CivitAI listed its LoRAs, a bare tag without a hash is most likely one of them under a file name.
    if (civitaiLoras.length && !hash) continue;
    res.push({ type: "lora", modelVersionId: null, modelName: name, modelVersionName: null, hash, weight: tag ? tag.weight : null });
  }

  // 5. Embeddings and VAE (resolved or ignored by the backend).
  for (const [name, hash] of tiHashes) {
    if (res.some((r) => r.type === "embed" && (r.hash === hash || (r.modelName && normName(r.modelName) === normName(name))))) continue;
    res.push({ type: "embed", modelVersionId: null, modelName: name, modelVersionName: null, hash, weight: null });
  }
  if (vaeHash && !res.some((r) => r.type === "vae")) {
    res.push({ type: "vae", modelVersionId: null, modelName: src.vaeName, modelVersionName: null, hash: vaeHash, weight: null });
  }
  return res;
}
