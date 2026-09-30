// Mock handlers for the app area (settings, hardware, engine, folders). See ./index.ts.
// RAM only; reset on reload.
//
// URL flags (screenshots / manual testing):
//   ?skipFirstRun   settings.firstRunDone = true and the engine is already installed
//   ?nogpu          no GPU detected (CPU only)
//   ?offline        Offline mode on
//   ?busygpu        other programs use 9 GB of graphics memory: a note while loading, and reading
//                   the prompt runs out of memory once per model (automatic retry, see generate.ts)
//   ?theme=dark     theme setting (light | dark | system)
//   ?failEngine     the first engine download fails half-way (to show retry)
//   ?slowhw         hardware detection takes ~4 s
//   ?lowdisk        only 9 GB free on the Data drive (see catalog.ts)
import type { MockTable } from "./index";
import { mockEmit } from "./index";
import type { AppInfo, CoreError, EngineStatus, GpuInfo, HardwareView, ModelsFolderInfo, ModelsFolderPreview, Settings } from "../types";
import { startMockDownload } from "./models";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const err = (code: string, message: string, details: string | null = null): CoreError => ({ code, message, details });

export function mockFlags() {
  const p = new URLSearchParams(typeof location !== "undefined" ? location.search : "");
  const theme = p.get("theme");
  return {
    skipFirstRun: p.has("skipFirstRun"),
    empty: p.has("empty"),
    noGpu: p.has("nogpu"),
    offline: p.has("offline"),
    failEngine: p.has("failEngine"),
    slowHw: p.has("slowhw"),
    lowDisk: p.has("lowdisk"),
    /** Other programs hold graphics memory: the first model load shows a note. */
    busyGpu: p.has("busygpu"),
    /** Adds an installed FLUX.2 klein model (reference picture in Create). */
    flux2: p.has("flux2"),
    theme: theme === "dark" || theme === "light" || theme === "system" ? (theme as Settings["theme"]) : null,
  };
}

// ---------------------------------------------------------------- settings
let settings: Settings | null = null;

export function mockSettings(): Settings {
  if (!settings) {
    const f = mockFlags();
    const initial: Settings = {
      offline: f.offline,
      gpu: "auto",
      vramOverrideGb: null,
      contentMode: "safe",
      showPaid: false,
      hideAnime: false,
      savedMetadata: "none",
      theme: f.theme ?? "system",
      addTriggerWords: true,
      firstRunDone: f.skipFirstRun,
      engineBackend: "auto",
      textEncoderOnCpu: "auto",
      modelsFolder: null,
      describeModel: "auto",
      improveModel: "auto",
    };
    settings = initial;
    return initial;
  }
  return settings;
}

/** Mirrors Rust `Settings::normalized`: unknown values fall back to defaults (never an error). */
function normalizeSettings(s: Settings): Settings {
  const gpu = String(s.gpu ?? "").trim().toLowerCase();
  const pick = <T extends string>(v: T, allowed: readonly string[], d: T): T => (allowed.includes(v) ? v : d);
  const vram = s.vramOverrideGb;
  return {
    ...s,
    gpu: gpu === "auto" || gpu === "cpu" || /^gpu:\d+$/.test(gpu) ? gpu : "auto",
    vramOverrideGb: vram != null && Number.isFinite(vram) && vram > 0 ? Math.min(vram, 1024) : null,
    contentMode: s.contentMode === ("include_18plus" as string) || s.contentMode === ("only_18plus" as string) ? "all" : pick(s.contentMode, ["safe", "all"], "safe"),
    savedMetadata: pick(s.savedMetadata, ["none", "settings"], "none"),
    theme: pick(s.theme, ["system", "light", "dark"], "system"),
    engineBackend: pick(s.engineBackend, ["auto", "cuda", "vulkan", "cpu"], "auto"),
    textEncoderOnCpu: pick(s.textEncoderOnCpu, ["auto", "on", "off"], "auto"),
    describeModel: String(s.describeModel ?? "").trim() || "auto",
    improveModel: String(s.improveModel ?? "").trim() || "auto",
  };
}

// ---------------------------------------------------------------- models folder
const DEFAULT_MODELS = "C:\\Users\\Alex\\AppData\\Local\\Pinhole\\Data\\models";

function modelsFolderInfo(): ModelsFolderInfo {
  const custom = mockSettings().modelsFolder;
  return { path: custom ?? DEFAULT_MODELS, custom: custom != null, problem: null };
}

// ---------------------------------------------------------------- hardware
function gpus(): GpuInfo[] {
  if (mockFlags().noGpu) return [];
  return [
    { index: 0, vendor: "nvidia", name: "NVIDIA GeForce RTX 5070 Ti", vramGb: 16 },
    { index: 1, vendor: "intel", name: "Intel(R) UHD Graphics 770", vramGb: 2 },
  ];
}

let hwReady = false;
let hwTimer: ReturnType<typeof setTimeout> | null = null;

/** Rust effective_hardware: "Force CPU" or engine backend cpu → no GPU at all. */
const cpuOnly = () => mockSettings().gpu === "cpu" || mockSettings().engineBackend === "cpu";

function selectedGpu(): GpuInfo | null {
  const s = mockSettings();
  const list = gpus();
  if (cpuOnly()) return null;
  const best = list.slice().sort((a, b) => b.vramGb - a.vramGb)[0] ?? null;
  if (s.gpu.startsWith("gpu:")) {
    const idx = Number(s.gpu.slice(4));
    return list.find((g) => g.index === idx) ?? best; // unknown index → best GPU (like Rust)
  }
  return best;
}

/** Effective VRAM after overrides (0 = CPU only). Used by the models/catalog mocks for Fits/Tight/Too big.
 *  Like Rust: whenever the backend is "cpu" there is no GPU and the VRAM override is ignored. */
export function effectiveVramGb(): number {
  if (backendFor(selectedGpu()) === "cpu") return 0;
  return mockSettings().vramOverrideGb ?? selectedGpu()?.vramGb ?? 0;
}

/** System RAM of the mock PC (?nogpu machines are small laptops). */
export function mockRamGb(): number {
  return mockFlags().noGpu ? 15.6 : 64;
}

function tierFor(vram: number): string {
  // Mirrors config/models.yaml → hardware_profiles.
  if (vram <= 7.9) return "low";
  if (vram <= 12.9) return "mid";
  if (vram <= 20.9) return "high";
  return "ultra";
}

function backendFor(g: GpuInfo | null): string {
  const s = mockSettings();
  if (cpuOnly()) return "cpu";
  if (s.engineBackend !== "auto") return s.engineBackend;
  if (!g) return "cpu";
  return g.vendor === "nvidia" ? "cuda" : "vulkan";
}

export function hardwareView(): HardwareView {
  if (!hwReady) {
    if (mockFlags().skipFirstRun && !mockFlags().slowHw) hwReady = true;
    else if (!hwTimer)
      hwTimer = setTimeout(
        () => {
          hwReady = true;
          mockEmit("hardware-ready", null);
        },
        mockFlags().slowHw ? 4000 : 1300,
      );
  }
  // Rust effective_hardware: the CPU backend means no GPU at all.
  const g = backendFor(selectedGpu()) === "cpu" ? null : selectedGpu();
  const vram = effectiveVramGb();
  return {
    detected: hwReady ? { gpus: gpus(), ramGb: mockRamGb(), cpuThreads: 24, os: "Windows 11 Pro 24H2" } : null,
    vramGb: hwReady ? vram : 0,
    gpu: hwReady ? g : null,
    backend: backendFor(hwReady ? g : null),
    tier: tierFor(hwReady ? vram : 0),
  };
}

// ---------------------------------------------------------------- engine
// Mirrors Rust engine_status: one engine build per backend. `backend` and
// `version` always name the build for the CURRENT backend (Settings / GPU), and
// `installed` says whether that build is on disk — switching backend in
// Settings reports "not installed" until it is downloaded. install_engine is a
// no-op when that build is already installed.
const ENGINE_VERSION = "master-3a9b1c2";
let installedBackends: Set<string> | null = null;
let engineFlags: Pick<EngineStatus, "installing" | "running" | "loading" | "loadedModelId" | "error" | "errorCode" | "errorDetails"> = {
  installing: false,
  running: false,
  loading: false,
  loadedModelId: null,
  error: null,
  errorCode: null,
  errorDetails: null,
};
let failEngineOnce: boolean | null = null;

function backends(): Set<string> {
  if (!installedBackends) installedBackends = new Set(mockFlags().skipFirstRun ? [backendFor(selectedGpu())] : []);
  return installedBackends;
}

function engineState(): EngineStatus {
  const backend = backendFor(selectedGpu());
  return { ...engineFlags, installed: backends().has(backend), version: ENGINE_VERSION, backend };
}

function setEngine(flags: Partial<typeof engineFlags>) {
  engineFlags = { ...engineFlags, ...flags };
  mockEmit("engine-status", engineState());
}

const MB = 1024 * 1024;

let pendingInstall: Promise<EngineStatus> | null = null;

/** Like Rust: a second call waits for the running install (then retries if it failed). */
async function installEngine(): Promise<EngineStatus> {
  await sleep(300);
  while (pendingInstall) await pendingInstall.catch(() => undefined);
  const backend = backendFor(selectedGpu());
  if (backends().has(backend)) return engineState();
  if (mockSettings().offline) throw err("offline", "Offline mode is on. Turn it off in Settings to browse or download.");
  pendingInstall = downloadEngine(backend).finally(() => {
    pendingInstall = null;
  });
  return pendingInstall;
}

function downloadEngine(backend: string): Promise<EngineStatus> {
  if (failEngineOnce == null) failEngineOnce = mockFlags().failEngine;
  const files =
    backend === "cuda"
      ? [
          { name: `sd-${ENGINE_VERSION}-bin-win-cuda12-x64.zip`, bytes: 214 * MB },
          { name: "cudart-sd-bin-win-cu12-x64.zip", bytes: 388 * MB },
        ]
      : backend === "vulkan"
        ? [{ name: `sd-${ENGINE_VERSION}-bin-win-vulkan-x64.zip`, bytes: 31 * MB }]
        : [{ name: `sd-${ENGINE_VERSION}-bin-win-cpu-x64.zip`, bytes: 9 * MB }];
  const label = `Image engine (${backend === "cuda" ? "NVIDIA CUDA" : backend === "vulkan" ? "Vulkan" : "CPU"})`;
  setEngine({ installing: true, error: null, errorCode: null, errorDetails: null });
  const fail = failEngineOnce;
  failEngineOnce = false;
  return new Promise<EngineStatus>((resolve, reject) => {
    startMockDownload(label, files, {
      kind: "engine",
      durationMs: 7000,
      failAt: fail ? 0.45 : undefined,
      onDone: () => {
        backends().add(backend);
        setEngine({ installing: false, error: null, errorCode: null, errorDetails: null });
        resolve(engineState());
      },
      onFail: (e) => {
        setEngine({ installing: false, error: e.message, errorCode: e.code, errorDetails: e.details ?? null });
        reject(e);
      },
      onCancel: () => {
        setEngine({ installing: false });
        reject(err("cancelled", "Cancelled"));
      },
    });
  });
}

// ---------------------------------------------------------------- local file picker (plugin-dialog)
let pickCount = 0;
const FAKE_PICKS = [
  "C:\\Users\\Alex\\Downloads\\flux1-dev-fp8.safetensors",
  "C:\\Users\\Alex\\Downloads\\ponyRealism_v22MainVAE.safetensors",
  "C:\\Users\\Alex\\Downloads\\dreamshaperXL_lightningDPMSDE.safetensors",
];

// ---------------------------------------------------------------- table
const table: MockTable = {
  app_info: async (): Promise<AppInfo> => ({
    version: "0.2.0",
    dataDir: "C:\\Users\\Alex\\AppData\\Local\\Pinhole\\Data",
    portable: false,
    os: "windows",
  }),
  get_settings: async () => ({ ...mockSettings() }),
  set_settings: async (a) => {
    await sleep(60);
    // Like Rust: the Models folder only changes by moving the models.
    const next = normalizeSettings({ ...mockSettings(), ...(a.settings as Settings), modelsFolder: mockSettings().modelsFolder });
    settings = next;
    return { ...next };
  },
  get_hardware: async () => {
    await sleep(80);
    return hardwareView();
  },
  open_data_folder: async () => undefined,
  models_folder_info: async (): Promise<ModelsFolderInfo> => modelsFolderInfo(),
  preview_models_folder: async (a): Promise<ModelsFolderPreview> => {
    await sleep(150);
    const folder = (a.folder as string | null) ?? null;
    if (folder != null && folder === mockSettings().modelsFolder) throw err("invalid", "Your models are already in that folder.");
    return { path: folder ?? DEFAULT_MODELS, isDefault: folder == null, files: 9, bytes: 31_400_000_000, existingModels: folder ? 1 : 0, sameDrive: false };
  },
  change_models_folder: async (a) => {
    const total = 31_400_000_000;
    for (let i = 1; i <= 10; i++) {
      await sleep(250);
      mockEmit("models-move-progress", { doneBytes: (total * i) / 10, totalBytes: total, fileName: "z_image_turbo-Q8_0.gguf" });
    }
    settings = { ...mockSettings(), modelsFolder: (a.folder as string | null) ?? null };
  },
  open_outputs_folder: async () => undefined,
  engine_status: async () => ({ ...engineState() }),
  engine_output: async () =>
    [
      "[INFO ] backend_fit.cpp:323  - auto-fit plan:",
      "[INFO ] backend_fit.cpp:326  -     CUDA0        NVIDIA GeForce RTX 4070          free  11500 MiB, budget  10988 MiB",
      "[INFO ] main.cpp:149  - listening on: http://127.0.0.1:5000",
    ].join("\n"),
  install_engine: () => installEngine(),
  "plugin:dialog|open": async (a) => {
    await sleep(300);
    // Only the "Add a file I already have" picker (model filters) gets a fake path; others look cancelled.
    const opts = a.options as { filters?: { extensions: string[] }[]; directory?: boolean } | undefined;
    // Settings → Models folder → "Change…" gets a fake shared-drive folder.
    if (opts?.directory) return "D:\\Shared\\Pinhole Models";
    const filters = (opts?.filters ?? []).flatMap((f) => f.extensions);
    if (!filters.includes("safetensors")) return null;
    return FAKE_PICKS[pickCount++ % FAKE_PICKS.length];
  },
};

export default table;
