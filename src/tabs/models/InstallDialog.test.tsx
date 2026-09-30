// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { InstallPlan, Settings } from "../../lib/types";

const api = vi.hoisted(() => ({
  planCivitaiInstall: vi.fn<(versionId: number, fileId: number | null) => Promise<InstallPlan>>(),
  getSettings: vi.fn(() => Promise.resolve({ addTriggerWords: true } as Settings)),
}));
vi.mock("../../lib/api", async (orig) => ({ ...(await orig<typeof import("../../lib/api")>()), ...api }));

import { InstallDialog } from "./InstallDialog";

const plan = (selected: number): InstallPlan => ({
  versionId: 7,
  modelName: "Test model",
  versionName: "v1",
  mainFile: { name: selected === 1 ? "full.safetensors" : "fp8.safetensors", sizeBytes: selected === 1 ? 12e9 : 6e9, format: "SafeTensor" },
  family: { familyId: "flux_dev", label: "FLUX.1 Dev" } as InstallPlan["family"],
  familyCandidates: [],
  components: [],
  totalDownloadBytes: selected === 1 ? 12e9 : 6e9,
  freeDiskBytes: 100e9,
  enoughDisk: true,
  vram: null,
  fit: null,
  licenseNote: null,
  isLora: false,
  trainedWords: [],
  blockedReason: null,
  needsApiKey: false,
  fileOptions: [1, 2].map((id) => ({
    fileId: id,
    name: id === 1 ? "full.safetensors" : "fp8.safetensors",
    sizeBytes: id === 1 ? 12e9 : 6e9,
    label: id === 1 ? "Full quality" : "Compact (FP8)",
    vram: null,
    fit: null,
    selected: id === selected,
  })),
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("InstallDialog size choice", () => {
  it("keeps the plan and the focused radio on screen while the new size loads", async () => {
    api.planCivitaiInstall.mockResolvedValueOnce(plan(1));
    render(<InstallDialog versionId={7} onClose={() => undefined} />);
    const compact = await screen.findByRole("radio", { name: /Compact \(FP8\)/ });

    let finish!: (p: InstallPlan) => void;
    api.planCivitaiInstall.mockImplementationOnce(() => new Promise((r) => (finish = r)));
    compact.focus();
    fireEvent.click(compact);

    // No skeleton: the same radio stays mounted, focused and checked; Install waits.
    expect(api.planCivitaiInstall).toHaveBeenLastCalledWith(7, 2);
    expect(screen.queryByText(/Checking the files on CivitAI/)).toBeNull();
    expect(screen.getByRole("radio", { name: /Compact \(FP8\)/ })).toBe(compact);
    expect(document.activeElement).toBe(compact);
    expect((compact as HTMLInputElement).checked).toBe(true);
    const install = screen.getByRole("button", { name: /Install/ }) as HTMLButtonElement;
    expect(install.disabled).toBe(true);

    await act(async () => finish(plan(2)));
    expect(screen.getByRole("radio", { name: /Compact \(FP8\)/ })).toBe(compact);
    expect((screen.getByRole("button", { name: /Install/ }) as HTMLButtonElement).disabled).toBe(false);
    expect(screen.getByText("fp8.safetensors", { selector: "span[title]" })).toBeTruthy();
  });

  it("shows the loading state again for another model", async () => {
    api.planCivitaiInstall.mockResolvedValue(plan(1));
    const { rerender } = render(<InstallDialog versionId={7} onClose={() => undefined} />);
    await screen.findByRole("radio", { name: /Compact \(FP8\)/ });
    api.planCivitaiInstall.mockImplementationOnce(() => new Promise(() => undefined));
    rerender(<InstallDialog versionId={8} onClose={() => undefined} />);
    expect(await screen.findByText(/Checking the files on CivitAI/)).toBeTruthy();
  });
});
