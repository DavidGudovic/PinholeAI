// IPC contract types. Mirrors the Rust structs (serde camelCase).
// Source of truth for the frontend ↔ backend boundary together with api.ts and
// docs/ARCHITECTURE.md. If you change a shape here, change the Rust side too.
//
// PRIVACY: types marked "prompt-bearing" contain user prompt text. Never log
// them (no console.*), never put them in localStorage/sessionStorage/IndexedDB,
// never put them in URLs or file names.

// ---------------------------------------------------------------- errors
export interface CoreError {
  /** offline | not_found | disk_space | vram | engine_missing | engine_failed |
   *  model_load | cancelled | unauthorized | hash_mismatch | invalid | network | io | internal.
   *  engine_missing = the image engine isn't installed (fix: installEngine).
   *  model_load = the engine couldn't load the model file (fix: Models → Installed). */
  code: string;
  /** Plain language, says what to do next. */
  message: string;
  /** Technical output for the "Details" toggle. */
  details?: string | null;
}

// ---------------------------------------------------------------- app / settings / hardware
export interface AppInfo {
  version: string;
  dataDir: string;
  portable: boolean;
  os: "windows" | "linux" | "macos" | string;
}

/** Safe mode: "safe" = On (hides models made for adults), "all" = Off. */
export type ContentMode = "safe" | "all";

export interface Settings {
  offline: boolean;
  /** auto | cpu | gpu:<index> */
  gpu: string;
  vramOverrideGb: number | null;
  contentMode: ContentMode;
  showPaid: boolean;
  /** Browse: hide anime models and add-ons. */
  hideAnime: boolean;
  /** none | settings */
  savedMetadata: "none" | "settings";
  theme: "system" | "light" | "dark";
  addTriggerWords: boolean;
  firstRunDone: boolean;
  /** auto | cuda | vulkan | cpu */
  engineBackend: string;
  /** Where the text encoder (reads the prompt) runs on a graphics-card engine:
   *  auto = graphics card, moved to the processor for a model (this app session)
   *  after it runs out of graphics memory; on = always the processor; off = never moved automatically
   *  (family flags such as --clip-on-cpu still apply). */
  textEncoderOnCpu: "auto" | "on" | "off";
  /** Models folder the user picked (absolute), null = Data/models. Read-only here:
   *  it only changes through changeModelsFolder, which moves the models. */
  modelsFolder: string | null;
}

export type Vendor = "nvidia" | "amd" | "intel" | "other";

export interface GpuInfo {
  index: number;
  vendor: Vendor;
  name: string;
  vramGb: number;
}

export interface HardwareView {
  /** null while detection is still running. */
  detected: {
    gpus: GpuInfo[];
    ramGb: number;
    cpuThreads: number;
    os: string;
  } | null;
  /** Effective VRAM after Settings overrides (0 = CPU only). */
  vramGb: number;
  /** Selected GPU (after override) or null for CPU. */
  gpu: GpuInfo | null;
  /** cuda | vulkan | cpu */
  backend: string;
  /** Hardware profile name from the registry: low | mid | high | ultra */
  tier: string;
}

// ---------------------------------------------------------------- engine
export interface EngineStatus {
  installed: boolean;
  installing: boolean;
  version: string | null;
  backend: string | null;
  running: boolean;
  loading: boolean;
  loadedModelId: string | null;
  /** Plain-language message of the last engine failure (CoreError.message). */
  error: string | null;
  /** CoreError.code of that failure (engine_failed | model_load | vram | …). */
  errorCode: string | null;
  /** Engine output for the "Details" toggle. */
  errorDetails: string | null;
  /** Plain-language note about the running engine (e.g. prompt read on the processor
   *  after running out of graphics memory; other programs using a lot of it). */
  note?: string | null;
}

// ---------------------------------------------------------------- downloads
export type DownloadState = "queued" | "downloading" | "verifying" | "done" | "failed" | "cancelled";

/** What a download group fetches (match on this, never on the label). */
export type DownloadKind = "engine" | "model" | "captioner" | "upscaler" | "appUpdate";

export interface GroupStatus {
  groupId: string;
  label: string;
  /** engine = image engine (installEngine); captioner = Describe model (+ its engine). null = untagged. */
  kind?: DownloadKind | null;
  state: DownloadState;
  currentFile: string | null;
  fileIndex: number;
  fileCount: number;
  downloadedBytes: number;
  totalBytes: number;
  error: string | null;
}

// ---------------------------------------------------------------- VRAM
export type Fit = "fits" | "tight" | "tooBig";

export interface VramNeed {
  gb: number;
  minGb: number;
  estimate: boolean;
  /** No usable GPU: gb/minGb are system RAM the model needs on the processor,
   *  and the badge is judged against RAM ("tight" = runs on the processor, slowly). */
  onCpu?: boolean;
}

// ---------------------------------------------------------------- models
export interface InstalledModel {
  id: string;
  friendlyName: string;
  familyId: string | null;
  familyLabel: string | null;
  /** "Realistic" | "Anime" | "3D" | "Illustration" | null */
  styleBadge: string | null;
  /** txt2img | img2img | inpaint | edit */
  modes: string[];
  isEditModel: boolean;
  /** Instruction edits can take a second image (registry `multi_ref`). */
  multiRef?: boolean;
  sizeBytes: number;
  vram: VramNeed | null;
  fit: Fit | null;
  /** Unix time in SECONDS of the last generation with this model. */
  lastUsed: number | null;
  /** Components the family needs that are not installed (labels). Empty = ready. */
  missingComponents: string[];
  licenseNote: string | null;
  civitaiModelId: number | null;
  civitaiVersionId: number | null;
  baseModel: string | null;
  /** "Q4", "Q3"… when the file is 4-bit or smaller (read from its header). */
  lowBit?: string | null;
}

export interface InstalledLora {
  id: string;
  friendlyName: string;
  familyId: string | null;
  baseModel: string | null;
  trainedWords: string[];
  sizeBytes: number;
  civitaiModelId: number | null;
  civitaiVersionId: number | null;
}

/** A helper model that isn't picked on Generate (Describe model, upscaler). */
export interface InstalledHelper {
  /** "describe" or the installed file id. */
  id: string;
  friendlyName: string;
  purpose: "describe" | "upscale";
  sizeBytes: number;
}

export interface ModelsFolderInfo {
  path: string;
  /** A folder the user picked (not Data/models). */
  custom: boolean;
  /** missing = drive not connected / mounted; readOnly = can't write (e.g. NTFS mounted read-only). */
  problem: "missing" | "readOnly" | null;
}

export interface ModelsFolderPreview {
  path: string;
  isDefault: boolean;
  files: number;
  /** Bytes that actually move (files the target already has aren't copied). */
  bytes: number;
  /** Models already in that folder, e.g. from Pinhole on your other operating system. */
  existingModels: number;
  sameDrive: boolean;
}

export interface ModelsMoveProgress {
  doneBytes: number;
  totalBytes: number;
  fileName: string;
}

export interface FamilyChoice {
  familyId: string;
  label: string;
}

/** Result of "Add a file I already have". */
export interface AddFileResult {
  /** Set when the file was registered. */
  model: InstalledModel | null;
  lora: InstalledLora | null;
  /** Set when detection is ambiguous: call confirmFamily(token, familyId). */
  needsChoice: { token: string; fileName: string; candidates: FamilyChoice[] } | null;
}

export interface DeletePreview {
  modelId: string;
  /** Files that will be removed, incl. components no other model uses. */
  files: { relPath: string; sizeBytes: number; reason: "model" | "orphanComponent" }[];
}

export interface RecommendedPick {
  /** realistic | realistic_detail (optional second Realistic card) | anime | edit | edit_alt (optional lighter edit model) | describe */
  role: string;
  roleLabel: string;
  /** null when nothing fits / no candidate verified yet. */
  title: string | null;
  familyId: string | null;
  goodAt: string | null;
  /** Only what is missing (shared components counted once). */
  downloadBytes: number;
  vram: VramNeed | null;
  fit: Fit | null;
  installed: boolean;
  /** Chosen quant (bf16 | q8_0 | q4_k) for registry models. */
  quant: string | null;
  licenseNote: string | null;
  unavailableReason: string | null;
  /** Plain words when this is a smaller version picked so it fits (or replaces a tight installed one). */
  note: string | null;
  /** An installed version of this model is a tight fit, and this smaller one fits. */
  replacesInstalled?: boolean;
}

export interface InstallStarted {
  groupId: string;
}

// ---------------------------------------------------------------- family UI / dials
export type Shape = "square" | "portrait" | "landscape" | "wide";
export type Quality = "fast" | "balanced" | "best";
export type GenMode = "txt2img" | "img2img" | "edit";

export interface FamilyUi {
  familyId: string;
  label: string;
  modes: string[];
  shapes: Record<string, [number, number]>;
  qualitySteps: [number, number, number];
  showStick: boolean;
  stickMapsTo: "cfg" | "guidance";
  stickRange: [number, number];
  stickDefault: number;
  usesNegativePrompt: boolean;
  defaultNegativePrompt: string | null;
  defaultSampler: string | null;
  defaultScheduler: string | null;
  defaultClipSkip: number | null;
  defaultFlowShift: number | null;
  defaultCfg: number;
  defaultGuidance: number | null;
  autoPromptPrefix: string | null;
  hiresAtBest: boolean;
  licenseNote: string | null;
  /** A dedicated edit model (Edit tab only). Families that also generate can still edit: `modes` has "edit". */
  isEditFamily: boolean;
  /** Show "Stay close to original" when editing with this family. */
  stayCloseShown: boolean;
  /** Default "Stay close to original" dial position (0…1) for edits. */
  stayCloseDefault: number;
}

export interface Dials {
  shape: Shape;
  quality: Quality;
  /** 0 (loose) … 1 (strict); for edit = "Stay close to original". */
  stick: number;
  count: 1 | 2 | 4;
}

/** prompt-bearing (negativePrompt). All fields optional = registry default. */
export interface FineTune {
  sampler?: string | null;
  scheduler?: string | null;
  steps?: number | null;
  cfg?: number | null;
  guidance?: number | null;
  seed?: number | null;
  flowShift?: number | null;
  clipSkip?: number | null;
  width?: number | null;
  height?: number | null;
  hires?: boolean | null;
  hiresScale?: number | null;
  hiresDenoise?: number | null;
  vaeTiling?: boolean | null;
  negativePrompt?: string | null;
  autoPromptPrefix?: boolean | null;
}

export interface LoraUse {
  loraId: string;
  weight: number;
  /** Trigger words picked on the add-on's chip. Unset = the default pick (see `pickedTriggerWords`). */
  words?: string[];
}

// ---------------------------------------------------------------- generation
/** prompt-bearing. */
export interface GenerateRequest {
  modelId: string;
  mode: GenMode;
  prompt: string;
  styleId: string | null;
  dials: Dials;
  fineTune: FineTune;
  loras: LoraUse[];
  /** Add LoRA trigger words to the prompt (in memory): each add-on's `words`, or all of them when unset. */
  addTriggerWords: boolean;
  /** img2img (Restyle): source image id in the session. */
  initImageId?: string | null;
  /** Restyle "How much to change": 0.35 | 0.55 | 0.75 */
  strength?: number | null;
  /** Instruction edit: reference images (ref_images). */
  refImageIds?: string[];
  /** Optional "Only change here" mask (session image id, white = change). */
  maskImageId?: string | null;
}

/** generated = txt2img/img2img/edit; upscaled = upscaleImage (model/seed/sampling copied from the source image). */
export type ResultKind = "generated" | "upscaled";

export interface ResultImage {
  id: string;
  kind?: ResultKind;
  width: number;
  height: number;
  seed: number;
  modelId: string;
  modelLabel: string;
  familyId: string;
  /** Settings summary (no prompt) shown on the card. */
  steps: number;
  cfg: number;
  guidance: number | null;
  sampler: string | null;
  scheduler: string | null;
  /** Parent image id for edit chains. */
  parentId: string | null;
}

export interface GenerateResult {
  images: ResultImage[];
}

export type GenPhase = "loadingModel" | "queued" | "generating" | "done" | "failed" | "cancelled";

export interface GenerationProgress {
  phase: GenPhase;
  modelLabel: string | null;
  queuePosition: number | null;
  step: number | null;
  totalSteps: number | null;
  elapsedMs: number;
  /** Plain-language note for this job (other programs using graphics memory, an
   *  automatic retry with memory-saving settings). */
  note?: string | null;
}

/** prompt-bearing. */
export interface FinalPromptPreview {
  prompt: string;
  negative: string | null;
}

export interface ImportedImage {
  id: string;
  width: number;
  height: number;
}

export interface SavedImage {
  path: string;
}

// ---------------------------------------------------------------- describe
export interface CaptionerStatus {
  /** Ready to describe without downloading anything. */
  available: boolean;
  /** reuse (edit model's Qwen2.5-VL) | default (small captioner) | null */
  source: "reuse" | "default" | null;
  /** Bytes to download to make the default captioner available. */
  downloadBytes: number;
  running: boolean;
}

export type DescribeStyle = "sentence" | "tags";

// ---------------------------------------------------------------- styles / presets
export interface Style {
  id: string;
  name: string;
  positive: string;
  negative: string | null;
  families: string[];
  thumbnail: string | null;
  builtin: boolean;
}

export interface PresetLora {
  loraId: string | null;
  civitaiVersionId: number | null;
  name: string;
  weight: number;
}

/** Stored fine-tune values: NO negative prompt, ever. */
export type PresetFineTune = Omit<FineTune, "negativePrompt" | "hiresScale" | "hiresDenoise">;

export interface Preset {
  id: string;
  name: string;
  family: string | null;
  modelId: string | null;
  civitaiVersionId: number | null;
  styleId: string | null;
  shape: Shape | null;
  quality: Quality | null;
  stick: number | null;
  count: number | null;
  fineTune: PresetFineTune;
  loras: PresetLora[];
  builtin: boolean;
}

// ---------------------------------------------------------------- catalog (CivitAI)
export type CatalogKind = "models" | "styleAddons";
export type PriceMode = "free" | "include" | "paid_only";

export interface BrowseQuery {
  kind: CatalogKind;
  /** Look key from catalog-filters.yaml (realistic | anime | illustration | three_d | brand) or null. */
  look: string | null;
  /** Tag keys from catalog-filters.yaml → tags; a model must match every one. */
  tags: string[];
  content: ContentMode;
  price: PriceMode;
  /** "Most Liked" | "Most Downloaded" | "Newest" */
  sort: string;
  /** Week | Month | Year | AllTime */
  period: string;
  commercialOnly: boolean;
  compatibleOnly: boolean;
  /** Hide models that are Too big for this machine. */
  runsOnMyCard?: boolean;
  /** Hide anime models and add-ons (tags, name words, anime-native base models). */
  hideAnime?: boolean;
  query: string;
  cursor: string | null;
}

export interface CatalogFilterOptions {
  looks: { key: string; label: string }[];
  /** Tags multi-select. `needsSafeModeOff`: the NSFW tag (finds only what Safe mode hides). */
  tags: { key: string; label: string; needsSafeModeOff: boolean }[];
  sorts: { label: string; api: string }[];
  periods: { label: string; api: string }[];
  content: { key: ContentMode; label: string }[];
  price: { key: PriceMode; label: string }[];
  defaultContent: ContentMode;
  defaultPrice: PriceMode;
  /** Opening sort / time (`api` values from `sorts` / `periods`). */
  defaultSort: string;
  defaultPeriod: string;
}

export interface CatalogCard {
  modelId: number;
  versionId: number;
  name: string;
  versionName: string;
  /** civitai type: Checkpoint | LORA */
  type: string;
  baseModel: string;
  familyId: string | null;
  styleBadge: string | null;
  creator: string | null;
  /** Preview URL — fetch bytes via fetchPreview(); never put it in an <img src>. */
  previewUrl: string | null;
  /** The preview comes from a video: `previewUrl` is a still frame of it (a bare video file URL is never fetched). */
  previewIsVideo: boolean;
  previewNsfw: boolean;
  modelNsfw: boolean;
  thumbsUpRatio: number | null;
  downloadCount: number;
  downloadBytes: number | null;
  vram: VramNeed | null;
  fit: Fit | null;
  earlyAccess: boolean;
  commercialOk: boolean;
  licenseNote: string | null;
  installed: boolean;
  /** Why it can't be installed (pickle only, scans failed…), or null. */
  blockedReason: string | null;
  /** "Compact (FP8)"… when a smaller file of this version was picked so it fits the card. */
  smallerFile?: string | null;
}

export interface BrowsePage {
  items: CatalogCard[];
  nextCursor: string | null;
  offline: boolean;
  /** True when client-side filtering hit the 5-extra-requests cap: show "Load more". */
  partial: boolean;
  /** CivitAI models looked at for this page. */
  checked: number;
  /** …hidden by Safe mode (models made for adults). */
  hiddenByContent: number;
  /** …hidden by Look, price, commercial use, kind or "Works with Pinhole". */
  hiddenByFilters: number;
  /** …hidden by "Runs on my card" (too big for this machine). */
  hiddenBySize?: number;
}

/** One preview image on a model's details page. */
export interface GalleryItem {
  index: number;
  /** Small rendition for the grid — fetch via fetchPreview(). */
  thumbUrl: string;
  /** Full-size image for "Edit this" — fetch via fetchPreview(). */
  fullUrl: string;
  width: number | null;
  height: number | null;
  nsfw: boolean;
  /** CivitAI generation data (prompt, settings, resources). prompt-bearing: memory only, never log or store. */
  generation: Record<string, unknown> | null;
}

export interface ModelGallery {
  items: GalleryItem[];
  /** Images left out because Safe mode is on (not rated PG, or not flagged safe). */
  hiddenNsfw: number;
  /** LoRA trigger words. */
  trainedWords: string[];
  /** What the creator wrote (raw CivitAI HTML, sanitize before showing). Absent when none or Safe mode hides it. */
  creatorNotes?: { model?: string | null; version?: string | null } | null;
  offline: boolean;
}

export interface InstallPlan {
  versionId: number;
  modelName: string;
  versionName: string;
  mainFile: { name: string; sizeBytes: number; format: string };
  family: FamilyChoice | null;
  /** Non-empty when the family is ambiguous: user must pick one. */
  familyCandidates: FamilyChoice[];
  components: { componentId: string; label: string; sizeBytes: number; installed: boolean }[];
  totalDownloadBytes: number;
  freeDiskBytes: number;
  enoughDisk: boolean;
  vram: VramNeed | null;
  fit: Fit | null;
  licenseNote: string | null;
  isLora: boolean;
  trainedWords: string[];
  /** Non-null → install is refused (unsafe format, failed scans…). */
  blockedReason: string | null;
  needsApiKey: boolean;
  /** Every installable file of the version; more than one = a size choice. */
  fileOptions?: PlanFileOption[];
  /** Label of the main file when a smaller one was picked so it fits the card. */
  smallerFile?: string | null;
}

export interface PlanFileOption {
  /** CivitAI file id: pass back to planCivitaiInstall / installCivitai. */
  fileId: number;
  name: string;
  sizeBytes: number;
  /** "Full quality" | "Compact (FP8)" | "Compact (Q4)" … */
  label: string;
  /** "Pictures can look grainy" and/or "Slower: part of it runs from system memory". */
  note?: string | null;
  vram: VramNeed | null;
  fit: Fit | null;
  selected: boolean;
}

// ---------------------------------------------------------------- paste from CivitAI
/** A resource from the "Civitai resources" JSON or hashes in pasted generation data. */
export interface PastedResource {
  /** checkpoint | lora | embed | vae | … */
  type: string;
  modelVersionId: number | null;
  modelName: string | null;
  modelVersionName: string | null;
  /** AutoV2 (10 hex) or full SHA-256. */
  hash: string | null;
  weight: number | null;
}

export interface ResolvedResource {
  resource: PastedResource;
  /** Installed model/LoRA id when we already have it. */
  installedId: string | null;
  /** Can be installed with installCivitai(versionId). */
  installableVersionId: number | null;
  displayName: string;
  familyId: string | null;
  downloadBytes: number | null;
  fit: Fit | null;
  /** Why it can't be used ("SD 1.5 LoRA doesn't work with SDXL models"…). */
  problem: string | null;
}

export interface ResolvedResources {
  checkpoint: ResolvedResource | null;
  loras: ResolvedResource[];
  ignored: ResolvedResource[];
}

// ---------------------------------------------------------------- updates (Rust pinhole_core::update)
/** How this copy can update itself. manual = open the release page (the .deb, dev builds). */
export type UpdateInstallMode = "installer" | "portable" | "appImage" | "manual";

export interface UpdateInfo {
  version: string;
  publishedAt: string | null;
  installMode: UpdateInstallMode;
  /** Download size for installMode (null for manual). */
  sizeBytes: number | null;
}

export interface UpdateCheck {
  currentVersion: string;
  /** null = already on the newest release. */
  update: UpdateInfo | null;
}
