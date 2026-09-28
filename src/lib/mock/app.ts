// Mock handlers for the app area (settings, hardware, engine, folders). See ./index.ts.
// RAM only; reset on reload.
//
// URL flags (screenshots / manual testing):
//   ?skipFirstRun   settings.firstRunDone = true and the engine is already installed
//   ?nogpu          no GPU detected (CPU only)
//   ?offline        Offline mode on
//   ?theme=dark     theme setting (light | dark | system)
//   ?failEngine     the first engine download fails half-way (to show retry)
//   ?slowhw         hardware detection takes ~4 s
//   ?lowdisk        only 9 GB free on the Data drive (see catalog.ts)
import type { MockTable } from "./index";
import { mockEmit } from "./index";
import type { AppInfo, CoreError, EngineStatus, GpuInfo, HardwareView, Settings } from "../types";
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
      savedMetadata: "none",
      theme: f.theme ?? "system",
      addTriggerWords: true,
      firstRunDone: f.skipFirstRun,
      engineBackend: "auto",
    };
    settings = initial;
    return initial;
  }
  return settings;
}

function validateSettings(s: Settings): Settings {
  const allowedGpu = s.gpu === "auto" || s.gpu === "cpu" || /^gpu:\d+$/.test(s.gpu);
  if (!allowedGpu) throw err("invalid", "That graphics card setting isn't valid. Pick one from the list.");
  if (!["auto", "cuda", "vulkan", "cpu"].includes(s.engineBackend)) throw err("invalid", "Unknown engine backend.");
  if (s.vramOverrideGb != null && (s.vramOverrideGb < 1 || s.vramOverrideGb > 192))
    throw err("invalid", "Graphics memory must be between 1 and 192 GB.");
  return s;
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

function selectedGpu(): GpuInfo | null {
  const s = mockSettings();
  const list = gpus();
  if (s.gpu === "cpu") return null;
  if (s.gpu.startsWith("gpu:")) {
    const idx = Number(s.gpu.slice(4));
    return list.find((g) => g.index === idx) ?? null;
  }
  return list.slice().sort((a, b) => b.vramGb - a.vramGb)[0] ?? null;
}

/** Effective VRAM after overrides (0 = CPU only). Used by the models/catalog mocks for Fits/Tight/Too big. */
export function effectiveVramGb(): number {
  const g = selectedGpu();
  if (!g) return 0;
  return mockSettings().vramOverrideGb ?? g.vramGb;
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
  const g = selectedGpu();
  const vram = effectiveVramGb();
  return {
    detected: hwReady ? { gpus: gpus(), ramGb: 64, cpuThreads: 24, os: "Windows 11 Pro 24H2" } : null,
    vramGb: hwReady ? vram : 0,
    gpu: hwReady ? g : null,
    backend: backendFor(hwReady ? g : null),
    tier: tierFor(hwReady ? vram : 0),
  };
}

// ---------------------------------------------------------------- engine
const ENGINE_VERSION = "master-3a9b1c2";
let engine: EngineStatus | null = null;
let failEngineOnce: boolean | null = null;

function engineState(): EngineStatus {
  if (!engine) {
    const installed = mockFlags().skipFirstRun;
    engine = {
      installed,
      installing: false,
      version: installed ? ENGINE_VERSION : null,
      backend: installed ? backendFor(selectedGpu()) : null,
      running: false,
      loading: false,
      loadedModelId: null,
      error: null,
    };
  }
  return engine;
}

function setEngine(e: EngineStatus) {
  engine = e;
  mockEmit("engine-status", e);
}

const MB = 1024 * 1024;

async function installEngine(): Promise<EngineStatus> {
  await sleep(300);
  if (mockSettings().offline) throw err("offline", "Offline mode is on. Turn it off in Settings to download the engine.");
  if (engineState().installing) throw err("invalid", "The engine is already downloading.");
  if (failEngineOnce == null) failEngineOnce = mockFlags().failEngine;
  const backend = backendFor(selectedGpu());
  const files =
    backend === "cuda"
      ? [
          { name: `sd-${ENGINE_VERSION}-bin-win-cuda12-x64.zip`, bytes: 214 * MB },
          { name: "cudart-sd-bin-win-cu12-x64.zip", bytes: 388 * MB },
        ]
      : backend === "vulkan"
        ? [{ name: `sd-${ENGINE_VERSION}-bin-win-vulkan-x64.zip`, bytes: 31 * MB }]
        : [{ name: `sd-${ENGINE_VERSION}-bin-win-cpu-x64.zip`, bytes: 9 * MB }];
  const label = `Image engine (${backend === "cuda" ? "CUDA" : backend === "vulkan" ? "Vulkan" : "CPU"})`;
  setEngine({ ...engineState(), installing: true, error: null });
  const fail = failEngineOnce;
  failEngineOnce = false;
  return new Promise<EngineStatus>((resolve, reject) => {
    startMockDownload(label, files, {
      durationMs: 7000,
      failAt: fail ? 0.45 : undefined,
      onDone: () => {
        setEngine({ ...engineState(), installed: true, installing: false, version: ENGINE_VERSION, backend, error: null });
        resolve(engineState());
      },
      onFail: (e) => {
        setEngine({ ...engineState(), installing: false, error: e.message });
        reject(e);
      },
      onCancel: () => {
        setEngine({ ...engineState(), installing: false });
        reject(err("cancelled", "Engine download cancelled."));
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
    version: "0.1.0",
    dataDir: "C:\\Users\\Alex\\AppData\\Local\\Pinhole\\Data",
    portable: false,
    os: "windows",
  }),
  get_settings: async () => ({ ...mockSettings() }),
  set_settings: async (a) => {
    await sleep(60);
    const next = validateSettings({ ...(a.settings as Settings) });
    settings = next;
    return { ...next };
  },
  get_hardware: async () => {
    await sleep(80);
    return hardwareView();
  },
  open_data_folder: async () => undefined,
  open_outputs_folder: async () => undefined,
  engine_status: async () => ({ ...engineState() }),
  install_engine: () => installEngine(),
  "plugin:dialog|open": async () => {
    await sleep(300);
    return FAKE_PICKS[pickCount++ % FAKE_PICKS.length];
  },
};

export default table;
