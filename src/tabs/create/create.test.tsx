// @vitest-environment jsdom
import { Profiler, type ReactNode } from "react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { GenerateRequest, GroupStatus, InstalledModel, Preset, ResultImage, Style } from "../../lib/types";

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
    captionerStatus: vi.fn(async () => ({ available: true, source: "default", downloadBytes: 0, running: false })),
    improvePrompt: vi.fn(async (p: string) => ({ text: `${p}, in soft light`, note: null })),
  };
});

const api = await import("../../lib/api");
const { installMocks } = await import("../../lib/mock");
const { AppProvider } = await import("../../lib/state/AppProvider");
const { StoreContext, createStore } = await import("../../lib/state/store");
const { ChoicesNote, GenerateLabel, PresetNoticeCard } = await import("./CreateTab");
const { SavePresetDialog } = await import("./PresetPicker");
const { FinalPromptPreview, FineTuneDrawer, LoraSection } = await import("./FineTune");
const { PromptBox } = await import("./PromptBox");
const { Results } = await import("./Results");
const tipModule = await import("./TipLine");
const { ReferenceSlot } = await import("./ReferenceSlot");
const { Dials } = await import("./Dials");
const { makeActions } = await import("../../lib/state/actions");
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

describe("model still needs parts", () => {
  it("Get missing parts opens Models on the Installed view", async () => {
    const { MissingPartsNote } = await import("./CreateTab");
    const { getLastView } = await import("../models/lib/session");
    const store = createStore();
    withApp(store, <MissingPartsNote missing={["VAE"]} />);
    expect(screen.getByText(/This model still needs VAE\./)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Get missing parts" }));
    expect(store.getState().tab).toBe("models");
    expect(getLastView()).toBe("installed");
  });
});

describe("Improve my prompt", () => {
  const box = (prompt: string) => {
    const store = createStore();
    store.dispatch({ type: "patchCreate", patch: { prompt } });
    withApp(store, <PromptBox ui={null} onOpenPaste={() => undefined} onApplyPasted={() => undefined} />);
    return store;
  };

  it("needs some text first", () => {
    box("");
    expect((screen.getByRole("button", { name: /Improve/ }) as HTMLButtonElement).disabled).toBe(true);
    // Still takes the pointer so its title explains why it is off.
    const cls = screen.getByRole("button", { name: /Improve/ }).className.split(" ");
    expect(cls).not.toContain("disabled:pointer-events-none");
    expect(cls).not.toContain("hover:bg-neutral-100");
  });

  it("replaces the prompt and Undo puts the original back", async () => {
    const store = box("a fox");
    fireEvent.click(screen.getByRole("button", { name: /Improve/ }));
    await waitFor(() => expect(store.getState().create.prompt).toBe("a fox, in soft light"));
    expect(api.improvePrompt).toHaveBeenCalledWith("a fox", null, [], "create");
    fireEvent.click(await screen.findByRole("button", { name: /Undo/ }));
    expect(store.getState().create.prompt).toBe("a fox");
  });

  it("drops the answer when the prompt was edited meanwhile", async () => {
    let done!: (v: string) => void;
    vi.mocked(api.improvePrompt).mockImplementationOnce(() => new Promise<{ text: string; note: string | null }>((r) => (done = (text: string) => r({ text, note: null }))));
    const store = box("a fox");
    fireEvent.click(screen.getByRole("button", { name: /Improve/ }));
    await waitFor(() => expect(api.improvePrompt).toHaveBeenCalled());
    store.dispatch({ type: "patchCreate", patch: { prompt: "a wolf" } });
    await act(async () => done("a fox, long text"));
    expect(store.getState().create.prompt).toBe("a wolf");
  });

  it("keeps the prompt and says so when the helper's answer was unusable", async () => {
    vi.mocked(api.improvePrompt).mockResolvedValueOnce({ text: "a fox", note: "The helper couldn't improve this one, so your prompt is unchanged." });
    const store = box("a fox");
    fireEvent.click(screen.getByRole("button", { name: /Improve/ }));
    await screen.findByText(/prompt is unchanged/);
    expect(store.getState().create.prompt).toBe("a fox");
    expect(screen.queryByRole("button", { name: /Undo/ })).toBeNull();
  });

  it("offers the helper model when it isn't installed", async () => {
    vi.mocked(api.captionerStatus).mockResolvedValueOnce({ available: false, source: null, downloadBytes: 2_750_000_000, running: false });
    const store = box("a fox");
    fireEvent.click(screen.getByRole("button", { name: /Improve/ }));
    await screen.findByRole("button", { name: /Get the helper/ });
    expect(api.improvePrompt).not.toHaveBeenCalled();
    expect(store.getState().create.prompt).toBe("a fox");
  });

  it("says why when the helper download failed and offers Try again", async () => {
    vi.mocked(api.captionerStatus).mockResolvedValueOnce({ available: false, source: null, downloadBytes: 0, running: false });
    const spy = vi.spyOn(api, "installCaptioner").mockResolvedValueOnce({ groupId: "helper-1" } as Awaited<ReturnType<typeof api.installCaptioner>>);
    const store = box("a fox");
    fireEvent.click(screen.getByRole("button", { name: /Improve/ }));
    fireEvent.click(await screen.findByRole("button", { name: /Get the helper/ }));
    await waitFor(() => expect(spy).toHaveBeenCalled());
    const failed: GroupStatus = {
      groupId: "helper-1",
      label: "Helper",
      kind: "captioner",
      state: "failed",
      currentFile: null,
      fileIndex: 0,
      fileCount: 1,
      downloadedBytes: 0,
      totalBytes: 1,
      error: "The download timed out — check your internet connection and try again.",
    };
    act(() => store.dispatch({ type: "download", status: failed }));
    expect((await screen.findByRole("alert")).textContent).toBe("The download timed out — check your internet connection and try again.");
    expect(screen.getByRole("button", { name: /Try again/ })).toBeTruthy();
    spy.mockRestore();
  });
});

describe("Results", () => {
  it("keeps the icon buttons together in one group that wraps as a unit", () => {
    withApp(storeWithResults(result("a", 64, 64)), <Results />);
    const icons = screen.getByTestId("result-icons");
    expect(icons.className.split(" ")).not.toContain("flex-wrap");
    for (const name of ["View full screen", "Copy image", "Remove from this session"]) {
      expect(within(icons).getByRole("button", { name })).toBeTruthy();
    }
    expect(within(icons).queryByRole("button", { name: /Describe/ })).toBeNull();
    expect(within(screen.getByTestId("result-actions")).getByRole("button", { name: /Describe/ })).toBeTruthy();
  });
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

  it("shows the small copy in the strip and the full picture on the stage", () => {
    const store = storeWithResults(result("a", 2048, 2048), result("b", 1024, 1024));
    store.dispatch({ type: "setThumb", id: "a", url: "blob:a", thumbUrl: "blob:a-thumb" });
    withApp(store, <Results />);
    const strip = screen.getByRole("listbox", { name: "Images in this session" });
    const [a, b] = within(strip).getAllByRole("option").map((o) => o.querySelector("img")!);
    expect(a.getAttribute("src")).toBe("blob:a-thumb");
    // No small copy yet: the full picture.
    expect(b.getAttribute("src")).toBe("blob:b");
    for (const img of [a, b]) {
      expect(img.getAttribute("decoding")).toBe("async");
      expect(img.getAttribute("loading")).toBe("lazy");
    }
    const shown = screen.getAllByRole("img").filter((i) => !strip.contains(i)).map((i) => i.getAttribute("src"));
    expect(shown).toContain("blob:a");
    expect(shown).not.toContain("blob:a-thumb");
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

  const batchRequest = (quality: "fast" | "balanced" | "best"): GenerateRequest => ({
    modelId: "m",
    mode: "txt2img",
    prompt: "a lighthouse",
    styleId: null,
    dials: { shape: "square", quality, stick: 0.5, count: 1 },
    fineTune: {},
    loras: [],
    addTriggerWords: true,
  });
  const storeWithBatch = (quality: "fast" | "balanced" | "best", r = result("n", 512, 512)) => {
    const store = createStore();
    store.dispatch({ type: "addResults", batch: { id: "b1", request: batchRequest(quality) }, images: [r], refs: [{ id: r.id, url: `blob:${r.id}`, width: r.width, height: r.height }] });
    return store;
  };

  it("offers Finish at Best quality first in Upscale for a picture made below Best", () => {
    const store = storeWithBatch("fast");
    withApp(store, <Results />);
    fireEvent.click(screen.getByRole("button", { name: /Upscale/ }));
    const items = screen.getAllByRole("menuitem");
    expect(items.map((i) => i.textContent?.replace(/(Best quality|Upscale \d×).*/, "$1"))).toEqual(["Finish at Best quality", "Upscale 2×", "Upscale 4×"]);
  });

  it("offers no Finish at Best quality for pictures made at Best, without settings, or upscaled", () => {
    for (const store of [storeWithBatch("best"), storeWithResults(result("n", 512, 512)), storeWithBatch("fast", { ...result("u", 1024, 1024, "o"), kind: "upscaled" })]) {
      withApp(store, <Results />);
      fireEvent.click(screen.getByRole("button", { name: /Upscale/ }));
      expect(screen.queryByRole("menuitem", { name: /Finish at Best/ })).toBeNull();
      cleanup();
    }
  });

  it("keeps Upscale available while a picture is being made, and says it waits", () => {
    const store = storeWithResults(result("n", 1024, 1024));
    store.dispatch({ type: "jobStart", kind: "create", at: 0, count: 1 });
    withApp(store, <Results />);
    const button = screen.getByRole("button", { name: /Upscale/ }) as HTMLButtonElement;
    expect(button.disabled).toBe(false);
    fireEvent.click(button);
    const items = screen.getAllByRole("menuitem") as HTMLButtonElement[];
    expect(items.map((i) => i.disabled)).toEqual([false, false]);
    expect(items[0].textContent).toContain("waits for the current job");
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

describe("reference picture", () => {
  const klein = { ...model, id: "k", friendlyName: "FLUX.2 klein", familyId: "flux2_klein_4b", modes: ["txt2img", "img2img", "edit"] } as InstalledModel;
  const sdxl = { ...model, id: "s", friendlyName: "Juggernaut", familyId: "sdxl", modes: ["txt2img", "img2img"] } as InstalledModel;

  it("is offered only for models that take one, and a session picture can be picked", () => {
    const store = storeWithResults(result("a", 64, 64));
    store.dispatch({ type: "setModels", models: [sdxl, klein] });
    const { rerender } = withApp(store, <ReferenceSlot model={sdxl} />);
    expect(screen.queryByRole("button", { name: /Add a reference picture/ })).toBeNull();

    rerender(
      <AppProvider store={store}>
        <ReferenceSlot model={klein} />
      </AppProvider>,
    );
    expect(screen.getByRole("button", { name: /Add a reference picture/ })).toBeTruthy();
    act(() => fireEvent.click(screen.getByRole("button", { name: "Use as the reference picture" })));
    expect(store.getState().create.refImageId).toBe("a");
    expect(screen.getByAltText("Reference picture")).toBeTruthy();
  });

  it("with a model that can't use it: says so, offers a model that can, and Generate explains", async () => {
    const store = storeWithResults(result("a", 64, 64));
    store.dispatch({ type: "setModels", models: [sdxl, klein] });
    store.dispatch({ type: "selectModel", modelId: "s" });
    store.dispatch({ type: "createSetRef", ref: store.getState().images.a });
    store.dispatch({ type: "patchCreate", patch: { prompt: "a lighthouse" } });
    withApp(store, <ReferenceSlot model={sdxl} />);
    expect(screen.getByText(/Juggernaut can't use a reference picture/)).toBeTruthy();

    const actions = makeActions(store);
    await expect(actions.generateCreate()).rejects.toMatchObject({ message: expect.stringContaining("can't use a reference picture") });

    fireEvent.click(screen.getByRole("button", { name: "Switch to FLUX.2 klein" }));
    expect(store.getState().create.modelId).toBe("k");
    fireEvent.click(screen.getByRole("button", { name: "Remove the reference picture" }));
    expect(store.getState().create.refImageId).toBeNull();
  });

  it("offers Same as reference in the Shape dial and picks it when the picture is added", () => {
    const store = storeWithResults(result("w", 1600, 900));
    withApp(store, <Dials ui={null} />);
    expect(screen.queryByRole("radio", { name: "Same as reference" })).toBeNull();

    act(() => store.dispatch({ type: "createSetRef", ref: store.getState().images.w }));
    const chip = screen.getByRole("radio", { name: "Same as reference" });
    expect(chip.getAttribute("aria-checked")).toBe("true");
    expect(chip.getAttribute("title")).toContain("1344×768");
    expect(screen.getByRole("radio", { name: "Square" }).getAttribute("aria-checked")).toBe("false");

    fireEvent.click(screen.getByRole("radio", { name: "Square" }));
    expect(store.getState().create.refShape).toBe(false);
    fireEvent.click(screen.getByRole("radio", { name: "Same as reference" }));
    expect(store.getState().create.refShape).toBe(true);
  });

  it("shows the one-time note about photos of people for a picture from the computer only", async () => {
    const store = storeWithResults({ ...result("a", 64, 64), origin: "generated" });
    store.dispatch({ type: "setModels", models: [sdxl, klein] });
    store.dispatch({ type: "setSettings", settings: { ...(await api.getSettings()), editNoticeSeen: false } });
    store.dispatch({ type: "createSetRef", ref: store.getState().images.a });
    withApp(store, <ReferenceSlot model={klein} />);
    expect(screen.queryByText(/Only use photos of people who have agreed/)).toBeNull();
    act(() => store.dispatch({ type: "createSetRef", ref: { id: "photo", url: "blob:photo", width: 64, height: 64 } }));
    expect(screen.getByText(/Only use photos of people who have agreed/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "OK" }));
    expect(screen.queryByText(/Only use photos of people who have agreed/)).toBeNull();
    await waitFor(() => expect(store.getState().settings?.editNoticeSeen).toBe(true));
  });

  it("Side by side shows the reference picture next to a result made with it", () => {
    globalThis.ResizeObserver ??= class {
      observe() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
    const store = createStore();
    const ref = { id: "r", url: "blob:r", width: 64, height: 96 };
    const made = result("a", 64, 64);
    const request = { refImageIds: ["r"] } as unknown as GenerateRequest;
    store.dispatch({ type: "addResults", batch: { id: "b", request }, images: [made], refs: [ref, { id: "a", url: "blob:a", width: 64, height: 64 }] });
    store.dispatch({ type: "addResults", batch: null, images: [result("c", 64, 64)], refs: [{ id: "c", url: "blob:c", width: 64, height: 64 }] });
    // The newest result (no reference) is shown first: no button.
    withApp(store, <Results />);
    expect(screen.queryByRole("button", { name: /Side by side/ })).toBeNull();

    act(() => store.dispatch({ type: "selectResult", id: "a" }));
    const button = screen.getByRole("button", { name: /Side by side/ });
    fireEvent.click(button);
    expect(button.getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByTestId("side-by-side")).toBeTruthy();
    fireEvent.click(button);
    expect(screen.queryByTestId("side-by-side")).toBeNull();
  });

  it("More like this holds Close to this one, Variations, Same character and On another model; Describe stays a button", () => {
    const store = storeWithResults(result("a", 64, 64));
    withApp(store, <Results />);
    expect(screen.queryByRole("button", { name: /Same character/ })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /More like this/ }));
    const items = screen.getAllByRole("menuitem");
    expect(items.map((i) => i.textContent?.match(/^(Close to this one|Variations|Same character|On another model…)/)?.[0])).toEqual(["Close to this one", "Variations", "Same character", "On another model…"]);
    // No batch for this result, so no Close to this one or Variations.
    expect((screen.getByRole("menuitem", { name: /Close to this one/ }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("menuitem", { name: /Variations/ }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: /Describe/ }));
    expect(store.getState().tab).toBe("describe");
    expect(store.getState().describe.imageId).toBe("a");
  });

  it("with no installed model that can use it: offers Edit", () => {
    const store = storeWithResults(result("a", 64, 64));
    store.dispatch({ type: "setModels", models: [sdxl] });
    store.dispatch({ type: "selectModel", modelId: "s" });
    store.dispatch({ type: "createSetRef", ref: store.getState().images.a });
    withApp(store, <ReferenceSlot model={sdxl} />);
    fireEvent.click(screen.getByRole("button", { name: "Use it in Edit" }));
    const s = store.getState();
    expect(s.tab).toBe("edit");
    expect(s.edit.mode).toBe("instruction");
    expect(s.edit.chain.map((n) => n.imageId)).toEqual(["a"]);
    expect(s.create.refImageId).toBeNull();
  });
});

describe("prompt box toolbar", () => {
  it("wraps onto a second line instead of squeezing Style under the Improve model picker", async () => {
    withApp(createStore(), <PromptBox ui={null} onOpenPaste={() => undefined} onApplyPasted={() => undefined} />);
    const improve = await screen.findByRole("button", { name: /Improve/ });
    const toolbar = improve.closest(".border-t") as HTMLElement;
    expect(toolbar.className).toContain("flex-wrap");
    expect(toolbar.querySelector("[aria-label^='Style']")).toBeTruthy();
  });
});

describe("Starter ideas", () => {
  it("fill an empty prompt and go away once there is text", () => {
    const store = createStore();
    withApp(store, <PromptBox ui={null} onOpenPaste={() => undefined} onApplyPasted={() => undefined} />);
    fireEvent.click(screen.getByRole("button", { name: "Watercolor fox" }));
    expect(store.getState().create.prompt).toMatch(/watercolor painting of a small fox/);
    expect(screen.queryByRole("button", { name: "Watercolor fox" })).toBeNull();
  });
});

describe("Named sizes", () => {
  it("set Width and Height in one click and mark the active one", () => {
    const store = createStore();
    withApp(store, <FineTuneDrawer ui={null} model={null} />);
    fireEvent.click(screen.getByRole("button", { name: /Fine-tune/ }));
    fireEvent.click(screen.getByRole("button", { name: "Phone" }));
    expect(store.getState().create.fineTune).toMatchObject({ width: 768, height: 1344 });
    expect(screen.getByRole("button", { name: "Phone" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByRole("button", { name: "Instagram" }).getAttribute("aria-pressed")).toBe("false");
  });
});

describe("Typing in the prompt", () => {
  it("does not re-render the dials, the Fine-tune drawer or the add-on list", async () => {
    const store = createStore();
    const renders = vi.fn();
    // The drawer stays closed: open, it shows the final prompt preview, which follows the prompt.
    withApp(
      store,
      <Profiler id="sidebar" onRender={renders}>
        <Dials ui={null} />
        <FineTuneDrawer ui={null} model={null} />
        <LoraSection model={null} />
      </Profiler>,
    );
    await flush();
    const before = renders.mock.calls.length;
    for (const prompt of ["a", "a f", "a fox"]) act(() => store.dispatch({ type: "patchCreate", patch: { prompt } }));
    expect(renders.mock.calls.length).toBe(before);
    // A dial change still shows up.
    act(() => store.dispatch({ type: "setDial", dial: "quality", value: "best" }));
    expect(renders.mock.calls.length).toBeGreaterThan(before);
  });
});

describe("Upscaler choice", () => {
  it("explains each upscaler, starts on Auto and saves the pick in Settings", async () => {
    const store = createStore();
    withApp(store, <FineTuneDrawer ui={null} model={null} />);
    fireEvent.click(screen.getByRole("button", { name: /Fine-tune/ }));
    const select = screen.getByRole("combobox", { name: "Upscaler" }) as HTMLSelectElement;
    expect(select.value).toBe("auto");
    expect(within(select).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "Auto: picks by picture style",
      "Photo: smooth (good for hair, can look waxy)",
      "Photo: skin texture (real skin, can make beards crunchy)",
      "Drawing: clean lines and flat colour",
    ]);
    fireEvent.change(select, { target: { value: "photo_texture" } });
    await waitFor(() => expect(store.getState().settings?.upscaler).toBe("photo_texture"));
    expect((await api.getSettings()).upscaler).toBe("photo_texture");
    fireEvent.click(screen.getByRole("button", { name: "Reset Upscaler (for Upscale)" }));
    await waitFor(() => expect(store.getState().settings?.upscaler).toBe("auto"));
  });
});

describe("Tip line", () => {
  const { TipLine, eligibleTips, resetSessionTip } = tipModule;
  beforeEach(() => resetSessionTip());

  it("only offers tips that fit", () => {
    expect(eligibleTips({ hasBatch: false, canReference: false })).not.toContain("variations");
    expect(eligibleTips({ hasBatch: false, canReference: false })).not.toContain("reference");
    expect(eligibleTips({ hasBatch: true, canReference: true })).toEqual(expect.arrayContaining(["variations", "reference"]));
  });

  it("shows one tip, and closing it keeps it closed for the session", () => {
    withApp(createStore(), <TipLine hasBatch />);
    expect(screen.getByRole("note").textContent).toMatch(/^Tip:/);
    fireEvent.click(screen.getByRole("button", { name: "Dismiss tip" }));
    expect(screen.queryByRole("note")).toBeNull();
  });

  it("stays away when tips are turned off", async () => {
    const store = createStore();
    store.dispatch({ type: "setSettings", settings: { ...(await api.getSettings()), showTips: false } });
    withApp(store, <TipLine hasBatch />);
    expect(screen.queryByRole("note")).toBeNull();
  });
});

describe("On another model", () => {
  const other = (id: string, extra: Partial<InstalledModel> = {}) => ({ ...model, id, friendlyName: `Model ${id}`, ...extra }) as InstalledModel;
  const request = (refImageIds?: string[]) =>
    ({ modelId: "m", mode: "txt2img", prompt: "p", styleId: null, dials: { shape: "square", quality: "balanced", stick: 0.5, count: 1 }, fineTune: {}, loras: [], addTriggerWords: true, ...(refImageIds ? { refImageIds } : {}) }) as GenerateRequest;
  const withBatch = (req: GenerateRequest, ...images: ResultImage[]) => {
    const store = createStore();
    store.dispatch({ type: "addResults", batch: { id: "b1", request: req }, images, refs: images.map((r) => ({ id: r.id, url: `blob:${r.id}`, width: r.width, height: r.height })) });
    return store;
  };

  it("lists the other installed models that fit and are set up", () => {
    const store = withBatch(request(), result("a", 64, 64));
    store.dispatch({
      type: "setModels",
      models: [model, other("fits", { fit: "fits" }), other("tight", { fit: "tight" }), other("big", { fit: "tooBig" }), other("part", { missingComponents: ["VAE"] })],
    });
    withApp(store, <Results />);
    fireEvent.click(screen.getByRole("button", { name: /More like this/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: /On another model/ }));
    const names = screen.getAllByRole("menuitem").map((i) => i.textContent ?? "");
    expect(names.some((n) => n.includes("Model fits"))).toBe(true);
    expect(names.some((n) => n.includes("Model tight"))).toBe(true);
    expect(names.some((n) => n.includes("Model big") || n.includes("Model part") || n.includes("Test model"))).toBe(false);
  });

  it("lists only models that take a reference picture when the picture used one", () => {
    const store = withBatch(request(["r"]), result("a", 64, 64));
    store.dispatch({ type: "setModels", models: [model, other("plain"), other("klein", { modes: ["txt2img", "edit"] })] });
    withApp(store, <Results />);
    fireEvent.click(screen.getByRole("button", { name: /More like this/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: /On another model/ }));
    const names = screen.getAllByRole("menuitem").map((i) => i.textContent ?? "");
    expect(names.some((n) => n.includes("Model klein"))).toBe(true);
    expect(names.some((n) => n.includes("Model plain"))).toBe(false);
  });

  it("opens the new picture side by side with the first, labelled by model", () => {
    const store = withBatch(request(), result("a", 64, 64));
    const made = { ...result("c", 64, 64), modelId: "o", modelLabel: "Other model" };
    store.dispatch({ type: "addResults", batch: { id: "b2", request: { ...request(), modelId: "o" }, compareWith: "a" }, images: [made], refs: [{ id: "c", url: "blob:c", width: 64, height: 64 }] });
    withApp(store, <Results />);
    expect(screen.getByTestId("side-by-side")).toBeTruthy();
    const toggle = screen.getByRole("button", { name: "Side by side with the picture from m" });
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(toggle);
    expect(screen.queryByTestId("side-by-side")).toBeNull();
  });
});

describe("choices in braces on the Generate button", () => {
  it("shows the count, and says when the cap trims it", () => {
    const store = createStore();
    store.dispatch({ type: "patchCreate", patch: { prompt: "a {red|blue|green} car" } });
    const ui = (queues: boolean) => (
      <AppProvider store={store}>
        <GenerateLabel queues={queues} />
        <ChoicesNote />
      </AppProvider>
    );
    const { container, rerender } = render(ui(false));
    expect(container.textContent).toBe("Generate 3");
    rerender(ui(true));
    expect(container.textContent).toBe("Add 3 to queue");
    act(() => store.dispatch({ type: "patchCreate", patch: { prompt: "{a|b|c|d|e} {1|2|3|4|5|6}" } }));
    expect(container.textContent).toContain("Add 16 to queue");
    expect(container.textContent).toContain("Makes the first 16 of 30 combinations.");
    act(() => store.dispatch({ type: "patchCreate", patch: { prompt: "a {cozy} cabin" } }));
    expect(container.textContent).toBe("Add to queue");
  });
});
