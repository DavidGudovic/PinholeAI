// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { DeletePreview, HardwareView, InstalledModel, RecommendedPick, Settings } from "../../lib/types";

const hw = vi.hoisted(() => ({ ready: [] as (() => void)[] }));
const api = vi.hoisted(() => ({
  listModels: vi.fn<() => Promise<InstalledModel[]>>(),
  listLoras: vi.fn(() => Promise.resolve([])),
  listHelpers: vi.fn(() => Promise.resolve([])),
  modelsFolderInfo: vi.fn(() => Promise.resolve({ path: "/models", problem: null })),
  getHardware: vi.fn(() => Promise.resolve(null as unknown as HardwareView)),
  getRecommended: vi.fn<() => Promise<RecommendedPick[]>>(() => Promise.resolve([])),
  previewDelete: vi.fn<(id: string) => Promise<DeletePreview>>(),
  onModelsChanged: vi.fn(() => Promise.resolve(() => undefined)),
  onHardwareReady: vi.fn((cb: () => void) => {
    hw.ready.push(cb);
    return Promise.resolve(() => undefined);
  }),
  onDownload: vi.fn(() => Promise.resolve(() => undefined)),
  listDownloads: vi.fn(() => Promise.resolve([])),
}));
vi.mock("../../lib/api", async (orig) => ({ ...(await orig<typeof import("../../lib/api")>()), ...api }));

import { emitSettingsChanged } from "../../settings/events";
import { InstalledView } from "./InstalledView";

const model = (fit: InstalledModel["fit"]): InstalledModel => ({
  id: "m1",
  friendlyName: "Test model",
  familyId: "sdxl",
  familyLabel: "SDXL",
  styleBadge: null,
  modes: ["txt2img"],
  isEditModel: false,
  sizeBytes: 6_000_000_000,
  vram: { gb: 8, estimate: false, onCpu: false } as InstalledModel["vram"],
  fit,
  lastUsed: null,
  missingComponents: [],
  licenseNote: null,
  civitaiModelId: null,
  civitaiVersionId: null,
  baseModel: null,
});

const settings = (patch: Partial<Settings>) => ({ gpu: "auto", vramOverrideGb: null, engineBackend: "auto", theme: "system", ...patch }) as Settings;

beforeEach(() => {
  hw.ready = [];
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("InstalledView", () => {
  it("re-reads the fit badges when the graphics card settings change or detection finishes", async () => {
    api.listModels.mockResolvedValue([model("fits")]);
    render(<InstalledView onBrowse={() => undefined} />);
    expect(await screen.findByText("Fits")).toBeTruthy();
    const calls = api.listModels.mock.calls.length;

    api.listModels.mockResolvedValue([model("tooBig")]);
    act(() => emitSettingsChanged(settings({ vramOverrideGb: 4 })));
    expect(await screen.findByText("Too big")).toBeTruthy();
    expect(api.listModels.mock.calls.length).toBe(calls + 1);
    // Only the hardware fields matter: a theme change doesn't refetch.
    act(() => emitSettingsChanged(settings({ vramOverrideGb: 4, theme: "dark" })));
    expect(api.listModels.mock.calls.length).toBe(calls + 1);
    // Recommended picks are sized against the hardware too.
    expect(api.getRecommended.mock.calls.length).toBeGreaterThanOrEqual(2);

    api.listModels.mockResolvedValue([model("tight")]);
    act(() => hw.ready.forEach((cb) => cb()));
    expect(await screen.findByText("Tight")).toBeTruthy();
  });

  it("offers Try again when the delete check fails", async () => {
    api.listModels.mockResolvedValue([model("fits")]);
    api.previewDelete.mockRejectedValueOnce({ code: "busy", message: "The models list is busy. Try again in a moment.", details: null });
    api.previewDelete.mockResolvedValueOnce({ modelId: "m1", files: [{ relPath: "models/test.safetensors", sizeBytes: 100, reason: "model" }] });
    render(<InstalledView onBrowse={() => undefined} />);
    fireEvent.click(await screen.findByRole("button", { name: "Delete Test model" }));
    fireEvent.click(await screen.findByRole("button", { name: "Try again" }));
    expect(await screen.findByText("models/test.safetensors")).toBeTruthy();
    expect(api.previewDelete).toHaveBeenCalledTimes(2);
    await waitFor(() => expect(screen.queryByRole("button", { name: "Try again" })).toBeNull());
  });
});
