// @vitest-environment jsdom
import { Profiler, type ReactNode } from "react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { GroupStatus, InstalledModel, Preset, ResultImage, Style } from "../../lib/types";

// Deferred IPC calls the tests resolve by hand; everything else goes to the mock backend.
const hold = <T,>() => {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
};
let install = hold<{ groupId: string }>();
let savePresetCall = hold<Preset>();
let saveImageCall = hold<{ path: string }>();
vi.mock("../../lib/api", async (orig) => {
  const real = await orig<typeof import("../../lib/api")>();
  return {
    ...real,
    installCivitai: vi.fn(() => install.promise),
    savePreset: vi.fn(() => savePresetCall.promise),
    saveImage: vi.fn(() => saveImageCall.promise),
    previewFinalPrompt: vi.fn(async () => ({ prompt: "p", negative: null })),
  };
});

const api = await import("../../lib/api");
const { installMocks } = await import("../../lib/mock");
const { AppProvider } = await import("../../lib/state/AppProvider");
const { StoreContext, createStore } = await import("../../lib/state/store");
const { PresetNoticeCard } = await import("./CreateTab");
const { SavePresetDialog } = await import("./PresetPicker");
const { FinalPromptPreview } = await import("./FineTune");
const { PromptBox } = await import("./PromptBox");
const { Results } = await import("./Results");
type Store = ReturnType<typeof createStore>;

beforeAll(async () => {
  await installMocks();
});
beforeEach(() => {
  install = hold();
  savePresetCall = hold();
  saveImageCall = hold();
  globalThis.URL.createObjectURL = vi.fn(() => "blob:x") as typeof URL.createObjectURL;
  globalThis.URL.revokeObjectURL = vi.fn();
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const withApp = (store: Store, ui: ReactNode) => render(<AppProvider store={store}>{ui}</AppProvider>);
const flush = () => act(() => new Promise((r) => setTimeout(r, 0)));

const model: InstalledModel = {
  id: "m",
  friendlyName: "Test model",
  familyId: null,
  familyLabel: "Test",
  styleBadge: null,
  modes: ["txt2img"],
  isEditModel: false,
  sizeBytes: 1,
  vram: null,
  fit: null,
  lastUsed: null,
  missingComponents: [],
  licenseNote: null,
  civitaiModelId: null,
  civitaiVersionId: null,
  baseModel: null,
} as unknown as InstalledModel;

const result = (id: string, width: number, height: number, parentId: string | null = null): ResultImage => ({
  id,
  width,
  height,
  seed: 1,
  modelId: "m",
  modelLabel: "m",
  familyId: "sdxl",
  steps: 20,
  cfg: 5,
  guidance: null,
  sampler: null,
  scheduler: null,
  parentId,
});

function storeWithResults(...images: ResultImage[]) {
  const store = createStore();
  store.dispatch({ type: "addResults", batch: null, images, refs: images.map((r) => ({ id: r.id, url: `blob:${r.id}`, width: r.width, height: r.height })) });
  return store;
}

describe("preset notice: Get the model", () => {
  const notice = {
    preset: { id: "p", name: "Moody portraits" } as Preset,
    app: { patch: {}, missingModel: { civitaiVersionId: 42, family: "sdxl" }, missingLoras: [], missingStyle: false },
  };
  const group = (state: GroupStatus["state"]): GroupStatus => ({
    groupId: "g1",
    label: "Model",
    state,
    currentFile: null,
    fileIndex: 0,
    fileCount: 1,
    downloadedBytes: 0,
    totalBytes: 0,
    error: null,
  });

  it("is busy while starting, follows the download, and offers Try again after it fails", async () => {
    const store = createStore();
    withApp(store, <PresetNoticeCard notice={notice} onDismiss={() => undefined} />);
    const button = () => screen.getByRole("button", { name: /Get the model|Downloading|Try again/ }) as HTMLButtonElement;
    fireEvent.click(button());
    expect(button().disabled).toBe(true);
    fireEvent.click(button());
    expect(api.installCivitai).toHaveBeenCalledTimes(1);

    await act(async () => install.resolve({ groupId: "g1" }));
    act(() => store.dispatch({ type: "download", status: group("downloading") }));
    await waitFor(() => expect(button().textContent).toContain("Downloading…"));
    expect(button().disabled).toBe(true);

    act(() => store.dispatch({ type: "download", status: group("failed") }));
    expect(button().textContent).toContain("Try again");
    expect(button().disabled).toBe(false);

    act(() => store.dispatch({ type: "download", status: group("done") }));
    expect(screen.getByText(/Downloaded/)).toBeTruthy();
  });

  it("never offers a second install when the download isn't listed", async () => {
    const store = createStore();
    withApp(store, <PresetNoticeCard notice={notice} onDismiss={() => undefined} />);
    const button = () => screen.getByRole("button", { name: /Get the model|Downloading|Try again/ }) as HTMLButtonElement;
    fireEvent.click(button());
    // Started, but no download event yet (and the refresh found nothing).
    await act(async () => install.resolve({ groupId: "g1" }));
    await flush();
    expect(button().textContent).toContain("Downloading…");
    expect(button().disabled).toBe(true);

    act(() => store.dispatch({ type: "download", status: group("done") }));
    // Clear finished downloads removes the entry; the card still says it's downloaded.
    act(() => store.dispatch({ type: "clearFinishedDownloads" }));
    expect(store.getState().downloads).toHaveLength(0);
    expect(screen.getByText(/Downloaded/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Get the model/ })).toBeNull();
    expect(api.installCivitai).toHaveBeenCalledTimes(1);
  });
});

describe("Save as preset", () => {
  it("saves once when Enter is pressed twice, and shows the quality label", async () => {
    const store = createStore();
    withApp(store, <SavePresetDialog open onClose={() => undefined} />);
    expect(screen.getByText(/Balanced/)).toBeTruthy();
    const name = screen.getByPlaceholderText("e.g. Moody portraits");
    fireEvent.change(name, { target: { value: "Night streets" } });
    fireEvent.submit(name.closest("form")!);
    fireEvent.submit(name.closest("form")!);
    expect(api.savePreset).toHaveBeenCalledTimes(1);
  });
});

describe("Final prompt preview", () => {
  it("refreshes when the selected style's words change", async () => {
    const store = createStore();
    const style: Style = { id: "s1", name: "Watercolor", positive: "watercolor", negative: null, families: [], thumbnail: null, builtin: false };
    store.dispatch({ type: "setStyles", styles: [style] });
    store.dispatch({ type: "patchCreate", patch: { prompt: "a lighthouse", styleId: "s1", modelId: "m" } });
    render(
      <StoreContext.Provider value={store}>
        <FinalPromptPreview ui={null} model={model} />
      </StoreContext.Provider>,
    );
    await waitFor(() => expect(api.previewFinalPrompt).toHaveBeenCalledTimes(1));
    act(() => store.dispatch({ type: "setStyles", styles: [{ ...style, positive: "oil painting" }] }));
    await waitFor(() => expect(api.previewFinalPrompt).toHaveBeenCalledTimes(2), { timeout: 2000 });
  });
});

describe("Paste as text", () => {
  it("keeps what was typed after the paste", async () => {
    const store = createStore();
    store.dispatch({ type: "patchCreate", patch: { prompt: "a castle" } });
    withApp(store, <PromptBox ui={null} onOpenPaste={() => undefined} onApplyPasted={() => undefined} />);
    const box = screen.getByLabelText("Prompt") as HTMLTextAreaElement;
    box.setSelectionRange(2, 8); // "castle" selected
    const data = "a ship\nNegative prompt: blurry\nSteps: 30, Sampler: Euler a, CFG scale: 7, Seed: 5, Size: 832x1216";
    fireEvent.paste(box, { clipboardData: { getData: () => data } });
    await screen.findByRole("button", { name: "Paste as text" });
    fireEvent.change(box, { target: { value: "a castle in fog" } });
    fireEvent.click(screen.getByRole("button", { name: "Paste as text" }));
    const prompt = store.getState().create.prompt;
    expect(prompt.startsWith("a castle in fog ")).toBe(true);
    expect(prompt).toContain(data);
  });
});

describe("Results", () => {
  it("doesn't re-render on progress ticks", () => {
    const store = storeWithResults(result("a", 64, 64));
    store.dispatch({ type: "jobStart", kind: "create", at: 0, count: 1 });
    const renders = vi.fn();
    withApp(
      store,
      <Profiler id="results" onRender={renders}>
        <Results />
      </Profiler>,
    );
    const before = renders.mock.calls.length;
    for (let step = 1; step <= 3; step++) {
      act(() =>
        store.dispatch({
          type: "jobProgress",
          progress: { phase: "generating", modelLabel: null, queuePosition: null, step, totalSteps: 8, elapsedMs: step * 300 },
        }),
      );
    }
    expect(renders.mock.calls.length).toBe(before);
  });

  it("offers no upscale that the upscaler would refuse", async () => {
    const store = storeWithResults(result("big", 2432, 1664));
    withApp(store, <Results />);
    fireEvent.click(screen.getByRole("button", { name: /Upscale/ }));
    const items = screen.getAllByRole("menuitem") as HTMLButtonElement[];
    expect(items.map((i) => i.disabled)).toEqual([true, true]);
    expect(items[0].textContent).toContain("Too large to upscale");
  });

  it("offers both upscales for a normal image", () => {
    const store = storeWithResults(result("n", 1024, 1024));
    withApp(store, <Results />);
    fireEvent.click(screen.getByRole("button", { name: /Upscale/ }));
    const items = screen.getAllByRole("menuitem") as HTMLButtonElement[];
    expect(items.map((i) => i.disabled)).toEqual([false, false]);
    expect(items[0].textContent).toContain("2048×2048");
  });

  it("names upscaled copies in the strip", () => {
    const store = storeWithResults(result("o", 64, 64), result("u", 256, 256, "o"));
    withApp(store, <Results />);
    expect(screen.getByRole("option", { name: /256×256.*upscaled/ })).toBeTruthy();
    expect(screen.queryByRole("option", { name: /64×64.*upscaled/ })).toBeNull();
  });

  it("saves once on a double-click and confirms with the toast only", async () => {
    const store = storeWithResults(result("s", 64, 64));
    withApp(store, <Results />);
    const save = screen.getByRole("button", { name: /^Save$/ });
    fireEvent.click(save);
    fireEvent.click(save);
    expect(api.saveImage).toHaveBeenCalledTimes(1);
    await act(async () => saveImageCall.resolve({ path: "/out/pinhole_1.png" }));
    await flush();
    expect(store.getState().toasts.map((t) => t.text)).toEqual(["Saved to /out/pinhole_1.png"]);
    expect(screen.queryByText(/Saved to/)).toBeNull();
    expect((screen.getByRole("button", { name: /^Save$/ }) as HTMLButtonElement).disabled).toBe(false);
  });
});
