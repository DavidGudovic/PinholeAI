// Mock handlers for the catalog area (CivitAI browser, previews, install plans, API key).
// See ./index.ts. No network: preview images are drawn on a canvas.
import type { MockTable } from "./index";
import type { BrowsePage, BrowseQuery, CatalogCard, CatalogFilterOptions, CoreError, InstallPlan, VramNeed } from "../types";
import { mockFlags, mockSettings } from "./app";
import { COMPONENTS, FAMILIES, installedComponents, sizeFor, isVersionInstalled, modelsChanged, registerLora, registerModel, startMockDownload } from "./models";

const MB = 1024 * 1024;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const err = (code: string, message: string, details: string | null = null): CoreError => ({ code, message, details });

// ---------------------------------------------------------------- fake CivitAI data
interface Entry {
  modelId: number;
  versionId: number;
  name: string;
  versionName: string;
  type: "Checkpoint" | "LORA";
  baseModel: string;
  familyId: string | null;
  /** Look keys (catalog-filters.yaml). First one drives the style badge. */
  looks: string[];
  creator: string;
  previewIsVideo: boolean;
  previewNsfw: boolean;
  modelNsfw: boolean;
  /** Not flagged by CivitAI but made for adults (suggestive tags / mostly R+ samples): Safe mode hides it. */
  suggestive: boolean;
  thumbsUpRatio: number;
  downloadCount: number;
  mainMb: number;
  gguf: boolean;
  earlyAccess: boolean;
  commercialOk: boolean;
  licenseNote: string | null;
  blockedReason: string | null;
  compatible: boolean;
  ambiguous: boolean;
  needsKey: boolean;
  trainedWords: string[];
  createdDaysAgo: number;
}

const BADGE: Record<string, string> = { realistic: "Realistic", anime: "Anime", illustration: "Illustration", three_d: "3D" };
const BASE_FAMILY: Record<string, string | null> = {
  "SDXL 1.0": "sdxl",
  "SDXL Lightning": "sdxl",
  Pony: "sdxl_pony",
  Illustrious: "sdxl_illustrious",
  NoobAI: "sdxl_illustrious",
  "SD 1.5": "sd15",
  "Flux.1 D": "flux1_dev",
  "Flux.1 S": "flux1_schnell",
  "Flux.1 Kontext": "flux1_kontext",
  ZImageTurbo: "z_image_turbo",
  Qwen: null,
  "SD 3.5 Large": null,
  HiDream: null,
};
const UNSUPPORTED = new Set(["SD 3.5 Large", "HiDream"]);

type Seed = Partial<Entry> & Pick<Entry, "name" | "type" | "baseModel" | "looks">;

const SEEDS: Seed[] = [
  { name: "Juggernaut XL", versionName: "Ragnarok", type: "Checkpoint", baseModel: "SDXL 1.0", looks: ["realistic"], versionId: 782002, modelId: 133005, creator: "KandooAI", thumbsUpRatio: 0.97, downloadCount: 1_240_000, mainMb: 6776, licenseNote: "CreativeML Open RAIL++-M" },
  { name: "RealVisXL V5.0", versionName: "V5.0 (BakedVAE)", type: "Checkpoint", baseModel: "SDXL 1.0", looks: ["realistic"], creator: "SG_161222", thumbsUpRatio: 0.96, downloadCount: 612_000, mainMb: 6617 },
  { name: "Z-Image Turbo Realism", versionName: "v1.2", type: "Checkpoint", baseModel: "ZImageTurbo", looks: ["realistic"], creator: "lumen_lab", thumbsUpRatio: 0.95, downloadCount: 48_300, mainMb: 12300, licenseNote: "Apache 2.0", createdDaysAgo: 4 },
  { name: "WAI-Illustrious-SDXL", versionName: "v14.0", type: "Checkpoint", baseModel: "Illustrious", looks: ["anime"], versionId: 1410435, modelId: 827184, creator: "WAI0731", thumbsUpRatio: 0.98, downloadCount: 890_000, mainMb: 6938, licenseNote: "Illustrious license" },
  { name: "DreamShaper XL", versionName: "Lightning DPM++ SDE", type: "Checkpoint", baseModel: "SDXL Lightning", looks: ["illustration", "realistic"], creator: "Lykon", thumbsUpRatio: 0.95, downloadCount: 540_000, mainMb: 6617, previewIsVideo: true },
  { name: "Pony Diffusion V6 XL", versionName: "V6 (start with this one)", type: "Checkpoint", baseModel: "Pony", looks: ["anime", "illustration"], creator: "PurpleSmartAI", thumbsUpRatio: 0.94, downloadCount: 1_020_000, mainMb: 6617, previewNsfw: true, licenseNote: "Fair AI Public License 1.0-SD" },
  { name: "FLUX.1 [dev] fp8", versionName: "fp8 e4m3fn", type: "Checkpoint", baseModel: "Flux.1 D", looks: ["realistic"], versionId: 691639, modelId: 618692, creator: "Black Forest Labs", thumbsUpRatio: 0.93, downloadCount: 402_000, mainMb: 11_900, commercialOk: false, licenseNote: "Non-commercial license" },
  { name: "FLUX.1 [schnell]", versionName: "Q8_0 GGUF", type: "Checkpoint", baseModel: "Flux.1 S", looks: ["realistic", "illustration"], creator: "city96", thumbsUpRatio: 0.9, downloadCount: 131_000, mainMb: 12_700, gguf: true, licenseNote: "Apache 2.0" },
  { name: "epiCRealism", versionName: "Natural Sin RC1", type: "Checkpoint", baseModel: "SD 1.5", looks: ["realistic"], creator: "epinikion", thumbsUpRatio: 0.96, downloadCount: 700_000, mainMb: 2034 },
  { name: "NoobAI-XL", versionName: "V-Pred 1.0", type: "Checkpoint", baseModel: "NoobAI", looks: ["anime"], creator: "L_A_X", thumbsUpRatio: 0.92, downloadCount: 210_000, mainMb: 6938 },
  { name: "Qwen Image Studio", versionName: "v2", type: "Checkpoint", baseModel: "Qwen", looks: ["realistic", "brand"], creator: "studio_q", thumbsUpRatio: 0.91, downloadCount: 22_400, mainMb: 11_600, gguf: true, ambiguous: true, licenseNote: "Apache 2.0", createdDaysAgo: 9 },
  { name: "Product Shot XL", versionName: "v3.1", type: "Checkpoint", baseModel: "SDXL 1.0", looks: ["brand", "realistic"], creator: "packshot", thumbsUpRatio: 0.89, downloadCount: 38_900, mainMb: 6617 },
  { name: "Clay Render 3D", versionName: "v2.0", type: "Checkpoint", baseModel: "SDXL 1.0", looks: ["three_d"], creator: "softbox", thumbsUpRatio: 0.93, downloadCount: 64_200, mainMb: 6617 },
  { name: "Toon Studio 3D", versionName: "v1.5", type: "Checkpoint", baseModel: "SDXL 1.0", looks: ["three_d", "illustration"], creator: "orbit", thumbsUpRatio: 0.9, downloadCount: 51_100, mainMb: 6617 },
  { name: "Classic Photo 1.4", versionName: "v1.4 (ckpt)", type: "Checkpoint", baseModel: "SD 1.5", looks: ["realistic"], creator: "oldtimer", thumbsUpRatio: 0.81, downloadCount: 18_000, mainMb: 4067, blockedReason: "Only available as an older file type (.ckpt) that can hide harmful code." },
  { name: "Fresh Upload XL", versionName: "v0.9", type: "Checkpoint", baseModel: "SDXL 1.0", looks: ["illustration"], creator: "newbie_42", thumbsUpRatio: 0.84, downloadCount: 310, mainMb: 6617, blockedReason: "CivitAI hasn't finished its safety scan for this file yet. Try again later.", createdDaysAgo: 1 },
  { name: "Hyper Realism Pro", versionName: "v7 Early Access", type: "Checkpoint", baseModel: "SDXL 1.0", looks: ["realistic"], creator: "prolens", thumbsUpRatio: 0.97, downloadCount: 9_800, mainMb: 6617, earlyAccess: true, needsKey: true, createdDaysAgo: 3 },
  { name: "Cinematic Frames XL", versionName: "v2 Early Access", type: "Checkpoint", baseModel: "SDXL 1.0", looks: ["realistic"], creator: "reelmaker", thumbsUpRatio: 0.94, downloadCount: 5_300, mainMb: 6617, earlyAccess: true, needsKey: true, createdDaysAgo: 6 },
  { name: "Noir Portrait XL", versionName: "v3", type: "Checkpoint", baseModel: "SDXL 1.0", looks: ["realistic"], creator: "noctis", thumbsUpRatio: 0.93, downloadCount: 250_000, mainMb: 6617, modelNsfw: true, previewNsfw: true },
  { name: "Anime Noir", versionName: "v5", type: "Checkpoint", baseModel: "Illustrious", looks: ["anime"], creator: "noctis", thumbsUpRatio: 0.92, downloadCount: 180_000, mainMb: 6938, modelNsfw: true, previewNsfw: true },
  { name: "Stable Diffusion 3.5 Large", versionName: "Large", type: "Checkpoint", baseModel: "SD 3.5 Large", looks: ["realistic", "illustration"], creator: "Stability AI", thumbsUpRatio: 0.82, downloadCount: 95_000, mainMb: 16_000, blockedReason: "Pinhole can't run this kind of model yet (SD 3.5).", licenseNote: "Stability Community License" },
  { name: "HiDream I1 Full", versionName: "fp8", type: "Checkpoint", baseModel: "HiDream", looks: ["realistic"], creator: "HiDream.ai", thumbsUpRatio: 0.86, downloadCount: 41_000, mainMb: 17_000, blockedReason: "Pinhole can't run this kind of model yet (HiDream)." },
  // Style add-ons (LoRAs)
  { name: "Film Photography Style XL", versionName: "v2", type: "LORA", baseModel: "SDXL 1.0", looks: ["realistic"], versionId: 991001, creator: "analog_anna", thumbsUpRatio: 0.97, downloadCount: 88_000, mainMb: 228, trainedWords: ["film photo", "kodak portra 400"] },
  { name: "Add More Details", versionName: "Detail Enhancer", type: "LORA", baseModel: "SD 1.5", looks: ["realistic", "illustration"], creator: "Lykon", thumbsUpRatio: 0.98, downloadCount: 1_300_000, mainMb: 36, trainedWords: [] },
  { name: "Pixel Art XL", versionName: "v1.1", type: "LORA", baseModel: "SDXL 1.0", looks: ["illustration"], creator: "nerijs", thumbsUpRatio: 0.96, downloadCount: 150_000, mainMb: 163, trainedWords: ["pixel art"] },
  { name: "Watercolor Wash", versionName: "Flux v1", type: "LORA", baseModel: "Flux.1 D", looks: ["illustration"], creator: "aquarelle", thumbsUpRatio: 0.95, downloadCount: 31_000, mainMb: 172, trainedWords: ["watercolor painting"], commercialOk: false, licenseNote: "Non-commercial license" },
  { name: "Soft Cel Anime", versionName: "v3", type: "LORA", baseModel: "Illustrious", looks: ["anime"], creator: "celshade", thumbsUpRatio: 0.94, downloadCount: 47_000, mainMb: 218, trainedWords: ["soft cel shading"] },
  { name: "Product Lighting Kit", versionName: "v1", type: "LORA", baseModel: "Flux.1 D", looks: ["brand"], creator: "packshot", thumbsUpRatio: 0.93, downloadCount: 12_900, mainMb: 172, trainedWords: ["studio product lighting", "white seamless"] },
  { name: "Isometric 3D Rooms", versionName: "v2", type: "LORA", baseModel: "SDXL 1.0", looks: ["three_d"], creator: "isoroom", thumbsUpRatio: 0.95, downloadCount: 27_000, mainMb: 218, trainedWords: ["isometric room"] },
  { name: "Retro Poster Art", versionName: "v1", type: "LORA", baseModel: "SDXL 1.0", looks: ["illustration"], creator: "poster_press", thumbsUpRatio: 0.92, downloadCount: 19_500, mainMb: 218, trainedWords: ["retro poster"] },
  { name: "Z-Image Portrait Boost", versionName: "v1", type: "LORA", baseModel: "ZImageTurbo", looks: ["realistic"], creator: "lumen_lab", thumbsUpRatio: 0.9, downloadCount: 6_200, mainMb: 162, trainedWords: [], createdDaysAgo: 5 },
  { name: "Relight (Qwen Edit)", versionName: "v1", type: "LORA", baseModel: "Qwen", looks: ["realistic", "brand"], creator: "studio_q", thumbsUpRatio: 0.91, downloadCount: 8_800, mainMb: 295, trainedWords: ["relight"], createdDaysAgo: 12 },
  { name: "Pose Study", versionName: "v2", type: "LORA", baseModel: "SDXL 1.0", looks: ["realistic"], creator: "atelier", thumbsUpRatio: 0.9, downloadCount: 60_000, mainMb: 218, modelNsfw: true, previewNsfw: true, trainedWords: ["pose study"] },
  { name: "Neon Cyberpunk", versionName: "v4", type: "LORA", baseModel: "SDXL 1.0", looks: ["illustration", "three_d"], creator: "neonrain", thumbsUpRatio: 0.93, downloadCount: 73_000, mainMb: 218, trainedWords: ["neon cyberpunk"], previewIsVideo: true },
];

// Filler names so the grid has several pages.
const FILLER_A = ["Velvet", "Golden Hour", "Soft Light", "Nordic", "Paper", "Midnight", "Coastal", "Studio", "Painterly", "Sunset", "Urban", "Porcelain", "Moss", "Vivid", "Quiet", "Chrome", "Pastel", "Analog", "Crystal", "Harbor"];
const FILLER_B = ["Realism", "Portraits", "Anime", "Dreams", "Illustrated", "Toon 3D", "Brand Studio", "Mix", "Photo", "Sketch"];
const LOOK_FOR_B: Record<string, string[]> = {
  Realism: ["realistic"],
  Portraits: ["realistic"],
  Anime: ["anime"],
  Dreams: ["illustration"],
  Illustrated: ["illustration"],
  "Toon 3D": ["three_d"],
  "Brand Studio": ["brand", "realistic"],
  Mix: ["illustration", "realistic"],
  Photo: ["realistic"],
  Sketch: ["illustration"],
};
const BASES = ["SDXL 1.0", "SDXL 1.0", "Illustrious", "Pony", "SD 1.5", "Flux.1 D", "ZImageTurbo"];

function mulberry32(seed: number) {
  return () => {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

let entries: Entry[] | null = null;

function allEntries(): Entry[] {
  if (entries) return entries;
  const rnd = mulberry32(7);
  const out: Entry[] = [];
  let id = 2_000_000;
  const complete = (s: Seed, i: number): Entry => {
    const fam = BASE_FAMILY[s.baseModel] ?? null;
    return {
      modelId: s.modelId ?? 400_000 + i * 37,
      versionId: s.versionId ?? id++,
      name: s.name,
      versionName: s.versionName ?? "v1.0",
      type: s.type,
      baseModel: s.baseModel,
      familyId: s.type === "LORA" ? fam : s.ambiguous ? null : fam,
      looks: s.looks,
      creator: s.creator ?? "someone",
      previewIsVideo: s.previewIsVideo ?? false,
      previewNsfw: s.previewNsfw ?? false,
      modelNsfw: s.modelNsfw ?? false,
      suggestive: s.suggestive ?? false,
      thumbsUpRatio: s.thumbsUpRatio ?? 0.9,
      downloadCount: s.downloadCount ?? 1000,
      mainMb: s.mainMb ?? 6617,
      gguf: s.gguf ?? false,
      earlyAccess: s.earlyAccess ?? false,
      commercialOk: s.commercialOk ?? !(s.licenseNote ?? "").toLowerCase().includes("non-commercial"),
      licenseNote: s.licenseNote ?? (fam && FAMILIES[fam]?.license) ?? "CreativeML Open RAIL++-M",
      blockedReason: s.blockedReason ?? null,
      compatible: !UNSUPPORTED.has(s.baseModel),
      ambiguous: s.ambiguous ?? false,
      needsKey: s.needsKey ?? false,
      trainedWords: s.trainedWords ?? [],
      createdDaysAgo: s.createdDaysAgo ?? Math.floor(rnd() * 400),
    };
  };
  SEEDS.forEach((s, i) => out.push(complete(s, i)));
  // Most seeds are "this month" so the default period isn't sparse.
  out.forEach((e, i) => {
    if (e.createdDaysAgo > 30 && i % 3 !== 0) e.createdDaysAgo = Math.floor(rnd() * 28);
  });
  let n = 0;
  const filler = (a: string, b: string, suffix: string) => {
    n += 1;
    const base = b === "Anime" ? (rnd() > 0.5 ? "Illustrious" : "Pony") : BASES[Math.floor(rnd() * BASES.length)];
    const isLora = rnd() > 0.72;
    out.push(
      complete(
        {
          name: `${a} ${b}${base.startsWith("SDXL") || base === "Pony" || base === "Illustrious" ? " XL" : ""}${suffix}`,
          versionName: `v${1 + Math.floor(rnd() * 6)}.${Math.floor(rnd() * 10)}`,
          type: isLora ? "LORA" : "Checkpoint",
          baseModel: base,
          looks: LOOK_FOR_B[b],
          creator: ["mira", "tobi", "kaz", "lena", "orbit", "fern"][Math.floor(rnd() * 6)],
          thumbsUpRatio: 0.78 + rnd() * 0.21,
          downloadCount: Math.floor(500 + rnd() * rnd() * 300_000),
          mainMb: isLora ? 120 + Math.floor(rnd() * 200) : base === "SD 1.5" ? 2034 : base.startsWith("Flux") ? 11_900 : base === "ZImageTurbo" ? 12_300 : 6617,
          previewNsfw: rnd() > 0.88,
          modelNsfw: rnd() > 0.9,
          suggestive: rnd() > 0.8,
          earlyAccess: rnd() > 0.9,
          trainedWords: isLora ? [b.toLowerCase()] : [],
          createdDaysAgo: rnd() > 0.35 ? Math.floor(rnd() * 28) : Math.floor(30 + rnd() * 500),
        },
        100 + n,
      ),
    );
  };
  for (const a of FILLER_A) {
    for (const b of FILLER_B) {
      if (rnd() > 0.34) continue;
      filler(a, b, "");
    }
  }
  // Older generations: several CivitAI pages, so infinite scroll and prefetch have work to do.
  for (const suffix of [" II", " Turbo", " Pro"]) {
    for (const a of FILLER_A) {
      for (const b of FILLER_B) {
        if (rnd() > 0.4) continue;
        filler(a, b, suffix);
      }
    }
  }
  entries = out;
  return out;
}

function vramFor(e: Entry): VramNeed | null {
  if (e.type === "LORA" || !e.compatible) return null;
  const fam = e.familyId ? FAMILIES[e.familyId] : null;
  if (!fam) return { gb: Math.round((e.mainMb / 1024 + 3.5) * 2) / 2, minGb: Math.round(e.mainMb / 1024 + 1), estimate: true };
  const gb = Math.max(fam.vram.min, Math.round((e.mainMb / 1024 + 3) * 2) / 2);
  return { gb, minGb: fam.vram.min, estimate: true };
}

function toCard(e: Entry, content?: BrowseQuery["content"]): CatalogCard {
  const { vram, fit } = sizeFor(vramFor(e), e.familyId, e.mainMb * MB);
  const badgeLook = e.looks.find((l) => BADGE[l]);
  return {
    modelId: e.modelId,
    versionId: e.versionId,
    name: e.name,
    versionName: e.versionName,
    type: e.type,
    baseModel: e.baseModel,
    familyId: e.familyId,
    styleBadge: badgeLook ? BADGE[badgeLook] : null,
    creator: e.creator,
    // Rust asks the CDN for CivitAI's own card rendition; videos come back as a still frame.
    previewUrl: `https://image.civitai.com/mock/${e.versionId}/${e.previewIsVideo ? "anim=false,transcode=true," : ""}width=450,optimized=true/${e.looks[0]}.jpeg`,
    previewIsVideo: e.previewIsVideo,
    // Safe mode previews are PG images (Rust: safe_filter.max_preview_level).
    previewNsfw: content === "safe" ? false : e.previewNsfw,
    modelNsfw: e.modelNsfw,
    thumbsUpRatio: e.thumbsUpRatio,
    downloadCount: e.downloadCount,
    downloadBytes: e.mainMb * MB,
    vram,
    fit,
    earlyAccess: e.earlyAccess,
    commercialOk: e.commercialOk,
    licenseNote: e.licenseNote,
    installed: isVersionInstalled(e.versionId),
    blockedReason: e.blockedReason,
  };
}

export function catalogEntryByVersion(versionId: number): CatalogCard | null {
  const e = allEntries().find((x) => x.versionId === versionId);
  return e ? toCard(e) : null;
}

// ---------------------------------------------------------------- browse
// Mirrors Rust (catalog-filters.yaml): CivitAI pages of API_LIMIT models by cursor; "server"
// filters (kind, search, baseModels, allowCommercialUse) narrow the list, then the client-side
// rules (Content, Look, Price) drop some and more pages are fetched until PAGE cards are
// found, at most 1 + MAX_EXTRA requests (then `partial` → "Load more").
const PAGE = 24;
const API_LIMIT = 50;
const MAX_EXTRA = 5;
const PERIOD_DAYS: Record<string, number> = { Week: 7, Month: 30, Year: 365 };

/** Safe mode hides these; the NSFW tag finds only these (Rust: model.nsfw or safe_filter rules). */
const isAdultEntry = (e: Entry) => e.modelNsfw || e.suggestive;
/** Tags multi-select (Rust: catalog-filters.yaml → tags). Subject tags are faked per entry. */
const mockTagMatches = (e: Entry, tag: string) =>
  tag === "nsfw" ? isAdultEntry(e) : tag === "edit" ? /\bedit\b|kontext/i.test(e.name) || e.baseModel === "Flux.1 Kontext" : (e.versionId + tag.length) % 3 === 0;

async function browse(q: BrowseQuery): Promise<BrowsePage> {
  await sleep(q.cursor ? 450 : 650);
  // Rust: no request; the cursor is handed back unchanged.
  if (mockSettings().offline) return { items: [], nextCursor: q.cursor, offline: true, partial: false, checked: 0, hiddenByContent: 0, hiddenByFilters: 0, hiddenBySize: 0 };
  if (q.query.toLowerCase() === "fail")
    throw err(
      "network",
      "Couldn't reach CivitAI. Check your internet connection, then try again.",
      "GET https://civitai.com/api/v1/models?limit=50&types=Checkpoint&query=fail\n→ error sending request: operation timed out after 30 s",
    );
  const text = q.query.trim().toLowerCase();
  const maxDays = PERIOD_DAYS[q.period];
  const commercial = q.commercialOnly || q.look === "brand";
  let server = allEntries().filter((e) => {
    if (q.kind === "models" ? e.type !== "Checkpoint" : e.type !== "LORA") return false;
    if (q.compatibleOnly && !e.compatible) return false;
    if (commercial && !e.commercialOk) return false;
    if (maxDays != null && e.createdDaysAgo > maxDays) return false;
    if (text && !`${e.name} ${e.creator} ${e.baseModel}`.toLowerCase().includes(text)) return false;
    return true;
  });
  if (q.sort === "Highest Rated") server = server.slice().sort((a, b) => b.thumbsUpRatio * Math.log10(b.downloadCount + 10) - a.thumbsUpRatio * Math.log10(a.downloadCount + 10));
  else if (q.sort === "Newest") server = server.slice().sort((a, b) => a.createdDaysAgo - b.createdDaysAgo);
  else server = server.slice().sort((a, b) => b.downloadCount - a.downloadCount);

  const out: BrowsePage = { items: [], nextCursor: null, offline: false, partial: false, checked: 0, hiddenByContent: 0, hiddenByFilters: 0, hiddenBySize: 0 };
  let offset = q.cursor ? Number(q.cursor) || 0 : 0;
  for (let requests = 1; ; requests++) {
    const batch = server.slice(offset, offset + API_LIMIT);
    offset += batch.length;
    for (const e of batch) {
      out.checked += 1;
      if (q.content === "safe" && isAdultEntry(e)) out.hiddenByContent += 1;
      else if (
        (q.look && !e.looks.includes(q.look)) ||
        !(q.tags ?? []).every((t) => mockTagMatches(e, t)) ||
        (q.price === "free" && e.earlyAccess) ||
        (q.price === "paid_only" && !e.earlyAccess)
      )
        out.hiddenByFilters += 1;
      else {
        const c = toCard(e, q.content);
        if (q.runsOnMyCard && c.fit === "tooBig") out.hiddenBySize! += 1;
        else out.items.push(c);
      }
    }
    out.nextCursor = offset < server.length ? String(offset) : null;
    if (out.items.length >= PAGE || !out.nextCursor) return out;
    if (requests >= 1 + MAX_EXTRA) return { ...out, partial: true };
    await sleep(250); // one more CivitAI request
  }
}

// ---------------------------------------------------------------- previews (drawn locally)
const PALETTES: Record<string, string[][]> = {
  realistic: [
    ["#1e3a5f", "#e8a87c", "#f6d6ad", "#3d5a4a", "#22333b"],
    ["#0f2027", "#2c5364", "#f4a261", "#264653", "#1b262c"],
    ["#4a3f35", "#d4a373", "#faedcd", "#606c38", "#283618"],
  ],
  anime: [
    ["#ffc8dd", "#bde0fe", "#a2d2ff", "#ffafcc", "#cdb4db"],
    ["#fde2e4", "#e2ece9", "#bee1e6", "#fad2e1", "#c5dedd"],
  ],
  illustration: [
    ["#264653", "#2a9d8f", "#e9c46a", "#f4a261", "#e76f51"],
    ["#3d405b", "#e07a5f", "#f2cc8f", "#81b29a", "#f4f1de"],
  ],
  three_d: [
    ["#22223b", "#4a4e69", "#9a8c98", "#c9ada7", "#f2e9e4"],
    ["#003049", "#d62828", "#f77f00", "#fcbf49", "#eae2b7"],
  ],
  brand: [["#f8f9fa", "#dee2e6", "#adb5bd", "#e76f51", "#264653"]],
};

function hashString(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

type Ctx = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;

function drawPreview(ctx: Ctx, w: number, h: number, look: string, seed: number) {
  const rnd = mulberry32(seed);
  const pals = PALETTES[look] ?? PALETTES.realistic;
  const pal = pals[Math.floor(rnd() * pals.length)];
  const sky = ctx.createLinearGradient(0, 0, 0, h);
  sky.addColorStop(0, pal[0]);
  sky.addColorStop(0.65, pal[1]);
  sky.addColorStop(1, pal[2]);
  ctx.fillStyle = sky;
  ctx.fillRect(0, 0, w, h);

  if (look === "three_d") {
    ctx.fillStyle = pal[4];
    ctx.globalAlpha = 0.35;
    ctx.fillRect(0, h * 0.68, w, h * 0.32);
    ctx.globalAlpha = 1;
    for (let i = 0; i < 3; i++) {
      const r = w * (0.12 + rnd() * 0.14);
      const x = w * (0.2 + rnd() * 0.6);
      const y = h * 0.68 - r * 0.6 + rnd() * 20;
      const g = ctx.createRadialGradient(x - r * 0.35, y - r * 0.4, r * 0.1, x, y, r);
      g.addColorStop(0, "#ffffff");
      g.addColorStop(0.25, pal[2 + (i % 3)]);
      g.addColorStop(1, pal[0]);
      ctx.fillStyle = "rgba(0,0,0,0.18)";
      ctx.beginPath();
      ctx.ellipse(x + r * 0.2, y + r * 0.95, r * 0.9, r * 0.22, 0, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.arc(x, y, r, 0, Math.PI * 2);
      ctx.fill();
    }
    return;
  }

  if (look === "brand") {
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, w, h);
    const floor = ctx.createLinearGradient(0, h * 0.55, 0, h);
    floor.addColorStop(0, "#f1f3f5");
    floor.addColorStop(1, "#dee2e6");
    ctx.fillStyle = floor;
    ctx.fillRect(0, h * 0.62, w, h * 0.38);
    const bx = w * 0.5;
    const bw = w * 0.22;
    ctx.fillStyle = "rgba(0,0,0,0.12)";
    ctx.beginPath();
    ctx.ellipse(bx, h * 0.8, bw * 0.9, 10, 0, 0, Math.PI * 2);
    ctx.fill();
    const body = ctx.createLinearGradient(bx - bw / 2, 0, bx + bw / 2, 0);
    body.addColorStop(0, pal[4]);
    body.addColorStop(0.35, "#4d7c8a");
    body.addColorStop(1, pal[4]);
    ctx.fillStyle = body;
    ctx.beginPath();
    ctx.roundRect(bx - bw / 2, h * 0.36, bw, h * 0.44, 18);
    ctx.fill();
    ctx.fillRect(bx - bw * 0.18, h * 0.26, bw * 0.36, h * 0.12);
    ctx.fillStyle = pal[3];
    ctx.fillRect(bx - bw / 2, h * 0.5, bw, h * 0.1);
    return;
  }

  // sun / moon
  ctx.fillStyle = look === "anime" ? "#ffffff" : pal[2];
  ctx.globalAlpha = 0.85;
  ctx.beginPath();
  ctx.arc(w * (0.25 + rnd() * 0.5), h * (0.2 + rnd() * 0.18), w * (0.08 + rnd() * 0.06), 0, Math.PI * 2);
  ctx.fill();
  ctx.globalAlpha = 1;

  if (look === "anime") {
    for (let i = 0; i < 5; i++) {
      const cx = rnd() * w;
      const cy = h * (0.15 + rnd() * 0.4);
      ctx.fillStyle = "rgba(255,255,255,0.75)";
      for (let k = 0; k < 4; k++) {
        ctx.beginPath();
        ctx.arc(cx + k * 18 - 27, cy + (k % 2) * 6, 18 + rnd() * 10, 0, Math.PI * 2);
        ctx.fill();
      }
    }
    ctx.fillStyle = "#ffffff";
    for (let i = 0; i < 26; i++) {
      ctx.globalAlpha = 0.4 + rnd() * 0.6;
      ctx.fillRect(rnd() * w, rnd() * h * 0.6, 2, 2);
    }
    ctx.globalAlpha = 1;
  }

  // layered hills / mountains
  for (let layer = 0; layer < 3; layer++) {
    const base = h * (0.55 + layer * 0.14);
    ctx.fillStyle = pal[(3 + layer) % pal.length];
    ctx.globalAlpha = look === "anime" ? 0.85 : 0.9 - layer * 0.1;
    ctx.beginPath();
    ctx.moveTo(0, h);
    ctx.lineTo(0, base);
    const peaks = look === "illustration" ? 3 : 6;
    for (let i = 0; i <= peaks; i++) {
      const x = (w / peaks) * i;
      const y = base - (look === "illustration" ? 40 : 60) * rnd() - (layer === 0 ? 40 : 0);
      if (look === "illustration") ctx.quadraticCurveTo(x - w / peaks / 2, y - 30, x, y);
      else ctx.lineTo(x, y);
    }
    ctx.lineTo(w, h);
    ctx.closePath();
    ctx.fill();
  }
  ctx.globalAlpha = 1;

  if (look === "realistic") {
    // soft vignette
    const v = ctx.createRadialGradient(w / 2, h / 2, w * 0.3, w / 2, h / 2, w * 0.8);
    v.addColorStop(0, "rgba(0,0,0,0)");
    v.addColorStop(1, "rgba(0,0,0,0.35)");
    ctx.fillStyle = v;
    ctx.fillRect(0, 0, w, h);
  }
}

async function fetchPreview(url: string): Promise<ArrayBuffer> {
  await sleep(150 + Math.random() * 450);
  if (mockSettings().offline) throw err("offline", "Offline mode is on.");
  const m = /mock\/(\d+)\/(?:[^/]+\/)?([a-z_]+)\./.exec(url);
  const seed = hashString(url);
  const look = m?.[2] ?? "realistic";
  const w = 320;
  const h = 400;
  if (typeof OffscreenCanvas !== "undefined") {
    const c = new OffscreenCanvas(w, h);
    drawPreview(c.getContext("2d")!, w, h, look, seed);
    const blob = await c.convertToBlob({ type: "image/png" });
    return blob.arrayBuffer();
  }
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  drawPreview(c.getContext("2d")!, w, h, look, seed);
  const blob = await new Promise<Blob>((res, rej) => c.toBlob((b) => (b ? res(b) : rej(err("internal", "Couldn't draw preview."))), "image/png"));
  return blob.arrayBuffer();
}

// ---------------------------------------------------------------- install plan / install
let hasKey = false;

/**
 * Big safetensors checkpoints also come as a compact FP8 file (like many CivitAI versions):
 * Rust picks the best one that fits unless the user chose one (SPEC §5.4 "Install").
 */
function fileChoices(e: ReturnType<typeof allEntries>[number], slug: string) {
  const full = { fileId: e.versionId * 10, name: `${slug}_${e.versionName.replace(/\s+/g, "_").replace(/[^A-Za-z0-9._-]/g, "")}.${e.gguf ? "gguf" : "safetensors"}`, mb: e.mainMb, label: "Full quality" };
  if (e.type === "LORA" || e.gguf || e.mainMb < 10000) return [full];
  const vram = vramFor(e);
  const fp8 = { fileId: e.versionId * 10 + 1, name: full.name.replace(/\.safetensors$/, "_fp8.safetensors"), mb: Math.round(e.mainMb / 2), label: "Compact (FP8)" };
  return [{ ...full, vram }, { ...fp8, vram: vram && { ...vram, gb: Math.round((vram.gb - e.mainMb / 2048) * 10) / 10, minGb: Math.max(1, vram.minGb - e.mainMb / 4096) } }];
}

function plan(versionId: number, chosenFile: number | null = null): InstallPlan {
  const e = allEntries().find((x) => x.versionId === versionId);
  if (!e) throw err("not_found", "This model is no longer on CivitAI.");
  const isLora = e.type === "LORA";
  const fam = e.familyId ? FAMILIES[e.familyId] : null;
  const have = installedComponents();
  const components = isLora || !fam ? [] : fam.components.map((c) => ({ componentId: c, label: COMPONENTS[c].label, sizeBytes: COMPONENTS[c].mb * MB, installed: have.has(c) }));
  const slug = e.name.replace(/[^A-Za-z0-9]+/g, "_").replace(/^_|_$/g, "");
  const choices = fileChoices(e, slug).map((c) => ({ ...c, ...sizeFor("vram" in c ? (c.vram ?? null) : vramFor(e), e.familyId, c.mb * MB) }));
  const picked = choices.find((c) => c.fileId === chosenFile) ?? (choices[0].fit === "fits" ? choices[0] : (choices.find((c) => c.fit === "fits") ?? choices[0]));
  const mainBytes = picked.mb * MB;
  const total = mainBytes + components.filter((c) => !c.installed).reduce((a, c) => a + c.sizeBytes, 0);
  const free = (mockFlags().lowDisk ? 9.2 : 214.6) * 1024 * MB;
  const { vram, fit } = picked;
  return {
    fileOptions: choices.map((c) => ({ fileId: c.fileId, name: c.name, sizeBytes: c.mb * MB, label: c.label, vram: c.vram, fit: c.fit, selected: c === picked })),
    smallerFile: chosenFile == null && picked !== choices[0] ? picked.label : null,
    versionId,
    modelName: e.name,
    versionName: e.versionName,
    mainFile: { name: picked.name, sizeBytes: mainBytes, format: e.gguf ? "GGUF" : "SafeTensor" },
    family: e.familyId ? { familyId: e.familyId, label: FAMILIES[e.familyId]?.label ?? e.familyId } : null,
    familyCandidates: e.ambiguous
      ? [
          { familyId: "qwen_image", label: "Qwen-Image (make new pictures)" },
          { familyId: "qwen_image_edit_2511", label: "Qwen Image Edit (change existing pictures)" },
        ]
      : [],
    components,
    totalDownloadBytes: total,
    freeDiskBytes: free,
    enoughDisk: total + 1024 * MB < free,
    vram,
    fit,
    licenseNote: e.licenseNote,
    isLora,
    trainedWords: e.trainedWords,
    blockedReason: e.blockedReason,
    needsApiKey: e.needsKey && !hasKey,
  };
}

async function install(versionId: number, familyId: string | null, fileId: number | null = null) {
  await sleep(350);
  if (mockSettings().offline) throw err("offline", "Offline mode is on. Turn it off in Settings to download models.");
  const p = plan(versionId, fileId);
  const e = allEntries().find((x) => x.versionId === versionId)!;
  if (p.blockedReason) throw err("invalid", p.blockedReason);
  if (!p.enoughDisk) throw err("disk_space", "Not enough free disk space. Delete a model you don't use, then try again.");
  if (p.needsApiKey)
    throw err("unauthorized", "CivitAI only lets signed-in users download this model. Add your CivitAI API key and try again.", "HTTP 401 Unauthorized from civitai.com/api/download/models/" + versionId);
  const fam = familyId ?? p.family?.familyId ?? null;
  if (!p.isLora && !fam) throw err("invalid", "Pick which kind of model this is first.");
  const missing = p.components.filter((c) => !c.installed);
  const groupId = startMockDownload(e.name, [{ name: p.mainFile.name, bytes: p.mainFile.sizeBytes }, ...missing.map((c) => ({ name: COMPONENTS[c.componentId].path.split("/").pop()!, bytes: c.sizeBytes }))], {
    onDone: () => {
      if (p.isLora)
        registerLora({
          friendlyName: e.name,
          familyId: e.familyId,
          baseModel: e.baseModel,
          trainedWords: e.trainedWords,
          sizeBytes: p.mainFile.sizeBytes,
          civitaiVersionId: versionId,
          relPath: `models/loras/${p.mainFile.name}`,
        });
      else
        registerModel({
          friendlyName: e.name,
          familyId: fam,
          familyLabel: fam ? FAMILIES[fam]?.label ?? fam : null,
          styleBadge: toCard(e).styleBadge,
          modes: fam ? FAMILIES[fam]?.modes ?? ["txt2img"] : ["txt2img"],
          isEditModel: !!(fam && FAMILIES[fam]?.edit),
          sizeBytes: p.mainFile.sizeBytes,
          vram: p.vram,
          licenseNote: e.licenseNote,
          civitaiModelId: e.modelId,
          civitaiVersionId: versionId,
          baseModel: e.baseModel,
          relPath: `models/${fam && (fam.startsWith("sdxl") || fam === "sd15") ? "checkpoints" : "diffusion"}/${p.mainFile.name}`,
        });
      modelsChanged();
    },
  });
  return { groupId };
}

// ---------------------------------------------------------------- table
const FILTERS: CatalogFilterOptions = {
  looks: [
    { key: "realistic", label: "Realistic" },
    { key: "anime", label: "Anime" },
    { key: "illustration", label: "Illustration" },
    { key: "three_d", label: "3D" },
    { key: "brand", label: "Brand & product" },
  ],
  tags: [
    { key: "edit", label: "Edit model", needsSafeModeOff: false },
    { key: "portraits", label: "Portraits", needsSafeModeOff: false },
    { key: "characters", label: "Characters", needsSafeModeOff: false },
    { key: "landscapes", label: "Landscapes", needsSafeModeOff: false },
    { key: "architecture", label: "Architecture", needsSafeModeOff: false },
    { key: "animals", label: "Animals", needsSafeModeOff: false },
    { key: "fantasy", label: "Fantasy", needsSafeModeOff: false },
    { key: "scifi", label: "Sci-fi", needsSafeModeOff: false },
    { key: "nsfw", label: "NSFW", needsSafeModeOff: true },
  ],
  sorts: [
    { label: "Top rated", api: "Highest Rated" },
    { label: "Most downloaded", api: "Most Downloaded" },
    { label: "Newest", api: "Newest" },
  ],
  periods: [
    { label: "This week", api: "Week" },
    { label: "This month", api: "Month" },
    { label: "This year", api: "Year" },
    { label: "All time", api: "AllTime" },
  ],
  content: [
    { key: "safe", label: "On" },
    { key: "all", label: "Off" },
  ],
  price: [
    { key: "free", label: "Free" },
    { key: "include", label: "Include early access (paid)" },
    { key: "paid_only", label: "Early access only" },
  ],
  defaultContent: "safe",
  defaultPrice: "free",
  defaultSort: "Most Downloaded",
  defaultPeriod: "AllTime",
};

const table: MockTable = {
  catalog_filters: async () => FILTERS,
  browse_catalog: (a) => browse(a.query as BrowseQuery),
  fetch_preview: (a) => fetchPreview(String(a.url)),
  plan_civitai_install: async (a) => {
    await sleep(500);
    if (mockSettings().offline) throw err("offline", "Offline mode is on. Turn it off in Settings to download models.");
    return plan(Number(a.versionId), a.fileId == null ? null : Number(a.fileId));
  },
  install_civitai: (a) => install(Number(a.versionId), (a.familyId as string | null) ?? null, a.fileId == null ? null : Number(a.fileId)),
  has_civitai_key: async () => hasKey,
  set_civitai_key: async (a) => {
    await sleep(250);
    const key = String(a.key ?? "").trim();
    if (key.length < 20 || /\s/.test(key))
      throw err("invalid", "That doesn't look like a CivitAI API key. Copy it again from civitai.com → Account settings → API keys.");
    hasKey = true; // the key itself is not kept, even in the mock
  },
  clear_civitai_key: async () => {
    await sleep(150);
    hasKey = false;
  },
};

export default table;
