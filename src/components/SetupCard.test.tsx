// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { EngineStatus, RecommendedPick, SafetyCheckStatus } from "../lib/types";

const api = vi.hoisted(() => ({
  engineStatus: vi.fn<() => Promise<EngineStatus>>(),
  safetyCheckStatus: vi.fn<() => Promise<SafetyCheckStatus>>(),
  getRecommended: vi.fn<() => Promise<RecommendedPick[]>>(),
  installEngine: vi.fn<() => Promise<EngineStatus>>(),
  installSafetyCheck: vi.fn<() => Promise<SafetyCheckStatus>>(),
  installRecommended: vi.fn<(role: string) => Promise<{ groupId: string }>>(),
}));
vi.mock("../lib/api", async (orig) => ({ ...(await orig<typeof import("../lib/api")>()), ...api }));

import { installMocks } from "../lib/mock";
import { AppProvider } from "../lib/state/AppProvider";
import { SetupCard, modelOffer, setupButtonLabel, type SetupItem } from "./SetupCard";

const GB = 1e9;
const engine = (installed: boolean): EngineStatus => ({
  installed,
  installing: false,
  version: "v1",
  backend: "cuda",
  running: false,
  loading: false,
  loadedModelId: null,
  error: null,
  errorCode: null,
  errorDetails: null,
  downloadBytes: installed ? 0 : 0.9 * GB,
});
const check = (ready: boolean): SafetyCheckStatus => ({ ready, downloading: false, downloadBytes: ready ? 0 : 1.2 * GB });
const pick = (over: Partial<RecommendedPick> = {}): RecommendedPick => ({
  role: "realistic",
  roleLabel: "Realistic",
  title: "Z-Image Turbo",
  familyId: "z_image",
  goodAt: null,
  downloadBytes: 6 * GB,
  vram: null,
  fit: "fits",
  installed: false,
  quant: null,
  licenseNote: null,
  unavailableReason: null,
  note: null,
  ...over,
});
const never = <T,>() => new Promise<T>(() => undefined);

beforeAll(async () => {
  await installMocks();
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function setup(engineInstalled: boolean, checkReady: boolean, picks: RecommendedPick[] = [pick()]) {
  api.engineStatus.mockResolvedValue(engine(engineInstalled));
  api.safetyCheckStatus.mockResolvedValue(check(checkReady));
  api.getRecommended.mockResolvedValue(picks);
  api.installEngine.mockImplementation(never);
  api.installSafetyCheck.mockImplementation(never);
  api.installRecommended.mockResolvedValue({ groupId: "g1" });
}

describe("setupButtonLabel", () => {
  const item = (key: SetupItem["key"], bytes: number, downloading = false): SetupItem => ({
    key,
    label: key === "model" ? "A model: Z-Image Turbo" : key,
    bytes,
    downloading,
  });
  it("says all, both, or names the one thing", () => {
    expect(setupButtonLabel([item("engine", GB), item("check", GB), item("model", 6 * GB)])).toBe("Get all (8.0 GB)");
    expect(setupButtonLabel([item("engine", GB), item("check", 1.2 * GB)])).toBe("Get both (2.2 GB)");
    expect(setupButtonLabel([item("check", 1.2 * GB)])).toBe("Get the safety check (1.2 GB)");
    expect(setupButtonLabel([item("engine", 0.9 * GB)])).toBe("Get the engine (900 MB)");
    expect(setupButtonLabel([item("model", 6 * GB)])).toBe("Get Z-Image Turbo (6.0 GB)");
    // What is already downloading isn't counted again.
    expect(setupButtonLabel([item("engine", GB, true), item("check", 1.2 * GB)])).toBe("Get the safety check (1.2 GB)");
    expect(setupButtonLabel([item("engine", GB, true)])).toBe("Downloading… (see Downloads)");
  });

  it("offers a model that fits, realistic first", () => {
    expect(modelOffer([pick({ role: "anime", title: "Anime" }), pick()])?.title).toBe("Z-Image Turbo");
    expect(modelOffer([pick({ fit: "tight" }), pick({ role: "anime", title: "Anime" })])?.title).toBe("Anime");
    expect(modelOffer([pick({ installed: true })])).toBeNull();
    expect(modelOffer(null)).toBeNull();
  });
});

describe("SetupCard", () => {
  it("lists the engine, the safety check and a model with sizes, and gets all of them with one button", async () => {
    setup(false, false);
    render(
      <AppProvider>
        <SetupCard needsModel />
      </AppProvider>,
    );
    const card = await screen.findByRole("region", { name: "Pinhole still needs" }, { timeout: 5000 });
    await within(card).findByText("A model: Z-Image Turbo");
    expect(within(card).getByText("Image engine")).toBeTruthy();
    expect(within(card).getByText("Safety check")).toBeTruthy();
    expect(within(card).getByText("1.2 GB")).toBeTruthy();
    fireEvent.click(within(card).getByRole("button", { name: "Get all (8.1 GB)" }));
    await waitFor(() => expect(api.installRecommended).toHaveBeenCalledWith("realistic"));
    expect(api.installEngine).toHaveBeenCalledTimes(1);
    expect(api.installSafetyCheck).toHaveBeenCalledTimes(1);
  }, 10_000);

  it("says Get both when a model is already installed", async () => {
    setup(false, false);
    render(
      <AppProvider>
        <SetupCard needsModel={false} />
      </AppProvider>,
    );
    const card = await screen.findByRole("region", { name: "Pinhole still needs" }, { timeout: 5000 });
    await within(card).findByRole("button", { name: "Get both (2.1 GB)" });
    expect(within(card).queryByText(/A model/)).toBeNull();
    expect(api.getRecommended).not.toHaveBeenCalled();
  }, 10_000);

  it("names the one thing left", async () => {
    setup(true, false);
    render(
      <AppProvider>
        <SetupCard needsModel={false} />
      </AppProvider>,
    );
    const card = await screen.findByRole("region", { name: "Pinhole still needs" }, { timeout: 5000 });
    fireEvent.click(await within(card).findByRole("button", { name: "Get the safety check (1.2 GB)" }));
    await waitFor(() => expect(api.installSafetyCheck).toHaveBeenCalledTimes(1));
    expect(api.installEngine).not.toHaveBeenCalled();
  }, 10_000);

  it("shows a failed model download at once while the engine still downloads, and doesn't ask twice", async () => {
    setup(false, false);
    let failModel: (e: unknown) => void = () => undefined;
    api.installRecommended.mockImplementation(() => new Promise((_, reject) => (failModel = reject)));
    render(
      <AppProvider>
        <SetupCard needsModel />
      </AppProvider>,
    );
    const card = await screen.findByRole("region", { name: "Pinhole still needs" }, { timeout: 5000 });
    await within(card).findByText("A model: Z-Image Turbo");
    fireEvent.click(within(card).getByRole("button", { name: /Get all/ }));
    // Everything was asked for: the button waits instead of asking again.
    const waiting = await within(card).findByRole("button", { name: /Downloading/ });
    expect((waiting as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(waiting);
    expect(api.installRecommended).toHaveBeenCalledTimes(1);
    expect(api.installSafetyCheck).toHaveBeenCalledTimes(1);
    failModel({ code: "disk", message: "Not enough free space for this model.", details: null });
    expect(await within(card).findByText(/Not enough free space/)).toBeTruthy();
    // The model can be asked for again on its own.
    expect(await within(card).findByRole("button", { name: "Get Z-Image Turbo (6.0 GB)" })).toBeTruthy();
  }, 10_000);

  it("stays hidden when the engine and the safety check are ready", async () => {
    setup(true, true);
    render(
      <AppProvider>
        <SetupCard needsModel />
      </AppProvider>,
    );
    await waitFor(() => expect(api.safetyCheckStatus).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByRole("region", { name: "Pinhole still needs" })).toBeNull();
  });
});
