// @vitest-environment jsdom
import { Profiler, useEffect, useImperativeHandle, type Ref } from "react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { CaptionerStatus, InstalledModel } from "../../lib/types";

const hold = <T,>() => {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
};
let maskExport = hold<Blob | null>();
// Whether the mask counts as painted as soon as it is switched on.
let paintOnShow = true;
// Whether the brush is on (the mask's `active`), as last rendered.
let maskActive = false;
let captioner: CaptionerStatus = { available: false } as CaptionerStatus;

const model = {
  id: "m",
  friendlyName: "Test model",
  familyId: null,
  familyLabel: "Test",
  styleBadge: null,
  modes: ["txt2img", "img2img"],
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

vi.mock("../../lib/api", async (orig) => {
  const real = await orig<typeof import("../../lib/api")>();
  return {
    ...real,
    listModels: vi.fn(async () => [model]),
    importImage: vi.fn(async () => ({ id: "mask", width: 8, height: 8 })),
    generate: vi.fn(() => new Promise(() => undefined)),
    captionerStatus: vi.fn(async () => captioner),
    describeImage: vi.fn(async () => "a lighthouse"),
    improvePrompt: vi.fn(async (p: string) => ({ text: `${p}. Warm street lights. Keep the composition unchanged.`, note: null })),
  };
});

// A canvas-free mask: painted as soon as it is switched on; its export waits for the test.
vi.mock("./MaskCanvas", () => ({
  MaskCanvas: ({ ref, active, onPaintedChange }: { ref?: Ref<unknown>; active: boolean; onPaintedChange: (b: boolean) => void }) => {
    useImperativeHandle(ref, () => ({ clear: () => undefined, exportPng: () => maskExport.promise }));
    maskActive = active;
    useEffect(() => {
      if (active && paintOnShow) onPaintedChange(true);
    }, [active, onPaintedChange]);
    return null;
  },
}));

const api = await import("../../lib/api");
const { installMocks } = await import("../../lib/mock");
const { AppProvider, runPrimaryAction } = await import("../../lib/state/AppProvider");
const { createStore } = await import("../../lib/state/store");
const { EditTab } = await import("./EditTab");
const { resetEditModelLine } = await import("./EditModelLine");
const { CompareView } = await import("./CompareView");
const { DescribeTab } = await import("../describe/DescribeTab");

beforeAll(async () => {
  await installMocks();
});
beforeEach(() => {
  maskExport = hold();
  paintOnShow = true;
  maskActive = false;
  captioner = { available: false } as CaptionerStatus;
  globalThis.URL.createObjectURL = vi.fn(() => "blob:x") as typeof URL.createObjectURL;
  globalThis.URL.revokeObjectURL = vi.fn();
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const flush = () => act(() => new Promise((r) => setTimeout(r, 0)));
const ref = (id: string) => ({ id, url: `blob:${id}`, width: 64, height: 64 });
const made = (id: string) => ({ id, width: 64, height: 64, seed: 1, modelId: "m", modelLabel: "M", familyId: "sdxl", steps: 1, cfg: 1, guidance: null, sampler: null, scheduler: null, parentId: null, origin: "generated" as const });
/** A store with Create results (newest first, like the strip). */
const withResults = (...ids: string[]) => {
  const store = createStore();
  store.dispatch({ type: "addResults", batch: null, images: ids.map(made), refs: ids.map(ref) });
  return store;
};
const sessionThumbs = (name: string) => screen.getAllByRole("button", { name }).map((b) => b.querySelector("img")?.getAttribute("src"));

describe("Edit tab", () => {
  it("shows the Edit notice once, for a picture from the computer only", async () => {
    const settings = { ...(await api.getSettings()), editNoticeSeen: false };
    const store = createStore();
    store.dispatch({ type: "setSettings", settings });
    store.dispatch({
      type: "addResults",
      batch: null,
      images: [{ id: "made", width: 64, height: 64, seed: 1, modelId: "m", modelLabel: "M", familyId: "sdxl", steps: 1, cfg: 1, guidance: null, sampler: null, scheduler: null, parentId: null, origin: "generated" }],
      refs: [ref("made")],
    });
    store.dispatch({ type: "editLoad", ref: ref("made") });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await flush();
    expect(screen.queryByText(/Only edit photos of people who have agreed/)).toBeNull();
    act(() => store.dispatch({ type: "editLoad", ref: ref("photo") }));
    expect(await screen.findByText(/Only edit photos of people who have agreed/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "OK" }));
    expect(screen.queryByText(/Only edit photos of people who have agreed/)).toBeNull();
    await waitFor(() => expect(store.getState().settings?.editNoticeSeen).toBe(true));
  });

  it("doesn't re-render while hidden during a Create job's progress ticks", async () => {
    const store = createStore();
    store.dispatch({ type: "editLoad", ref: ref("a") });
    const renders = vi.fn();
    render(
      <AppProvider store={store}>
        <Profiler id="edit" onRender={renders}>
          <EditTab />
        </Profiler>
      </AppProvider>,
    );
    await flush();
    await flush();
    act(() => store.dispatch({ type: "jobStart", kind: "create", at: 0, count: 1 }));
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

  it("Fix details uses the brush without a toggle and sends the mask", async () => {
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(1));
    act(() => {
      store.dispatch({ type: "editLoad", ref: ref("a") });
    });
    fireEvent.click(await screen.findByRole("radio", { name: "Fix details" }));
    await flush();
    expect(screen.queryByRole("switch", { name: /Only change here/ })).toBeNull();
    expect(screen.getByLabelText(/What is it\?/)).toBeTruthy();
    // No prompt needed: the painted spot is enough.
    const fix = screen.getByRole("button", { name: /^Fix details/ });
    expect((fix as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(fix);
    await act(async () => maskExport.resolve(new Blob([new Uint8Array(4)])));
    await flush();
    await waitFor(() => expect(api.generate).toHaveBeenCalledTimes(1));
    const req = vi.mocked(api.generate).mock.calls[0][0];
    expect(req).toMatchObject({ mode: "img2img", fixDetails: true, maskImageId: "mask", initImageId: "a", prompt: "" });
  });

  it("Fix details with nothing painted adds detail to the faces, without a mask", async () => {
    paintOnShow = false;
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(1));
    act(() => {
      store.dispatch({ type: "editLoad", ref: ref("a") });
    });
    fireEvent.click(await screen.findByRole("radio", { name: "Fix details" }));
    await flush();
    const run = screen.getByRole("button", { name: /^Add detail/ });
    expect((run as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(run);
    await flush();
    await waitFor(() => expect(api.generate).toHaveBeenCalledTimes(1));
    const req = vi.mocked(api.generate).mock.calls[0][0];
    expect(req).toMatchObject({ mode: "img2img", fixDetails: true, maskImageId: null, initImageId: "a" });
  });

  it("Extend picks a new shape and side, has no brush and sends the canvas", async () => {
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(1));
    act(() => {
      store.dispatch({ type: "editLoad", ref: ref("a") });
    });
    fireEvent.click(await screen.findByRole("radio", { name: "Extend" }));
    await flush();
    expect(screen.queryByRole("switch", { name: /Only change here/ })).toBeNull();
    expect(screen.queryByText(/How much to change/)).toBeNull();
    // The picture is square already: Square can't be picked.
    expect((screen.getByRole("radio", { name: "Square" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("radio", { name: "Wide" }));
    fireEvent.click(await screen.findByRole("radio", { name: "Right" }));
    expect(screen.getByLabelText("New space")).toBeTruthy();
    const go = screen.getByRole("button", { name: /^Extend/ });
    expect((go as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(go);
    await waitFor(() => expect(api.generate).toHaveBeenCalledTimes(1));
    const req = vi.mocked(api.generate).mock.calls[0][0];
    expect(req).toMatchObject({ mode: "img2img", initImageId: "a", strength: 1, maskImageId: null, prompt: "" });
    expect(req.extend).toMatchObject({ height: 64, left: 0, top: 0 });
    expect(req.extend!.width).toBeGreaterThan(100);
  });

  it("Try again on an Extend redoes it from the smaller step before", async () => {
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(1));
    const meta = { id: "b", kind: "generated", width: 114, height: 64, seed: 1, modelId: "m", modelLabel: "Test model", familyId: "sdxl", steps: 8, cfg: 5, guidance: null, sampler: null, scheduler: null, parentId: "a" } as const;
    act(() => {
      store.dispatch({ type: "editLoad", ref: ref("a") });
      store.dispatch({ type: "editPush", ref: { id: "b", url: "blob:b", width: 114, height: 64 }, meta, after: 0 });
    });
    // As after extending to Wide: the settings stay.
    act(() => store.dispatch({ type: "patchEdit", patch: { mode: "extend", extendTo: "wide" } }));
    await flush();
    // The shown result is wide already, but the step it came from isn't.
    expect((screen.getByRole("button", { name: /^Extend/ }) as HTMLButtonElement).disabled).toBe(true);
    const again = screen.getByRole("button", { name: "Try again" });
    expect((again as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(again);
    await waitFor(() => expect(api.generate).toHaveBeenCalledTimes(1));
    const req = vi.mocked(api.generate).mock.calls[0][0];
    expect(req).toMatchObject({ initImageId: "a", extend: { height: 64 } });
  });

  it("runs one edit when Restyle is pressed twice while the mask is exported", async () => {
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(1));
    act(() => {
      store.dispatch({ type: "editLoad", ref: ref("a") });
      store.dispatch({ type: "patchEdit", patch: { mode: "restyle", restylePrompt: "watercolor" } });
    });
    fireEvent.click(await screen.findByRole("switch", { name: /Only change here/ }));
    await flush();
    const restyle = screen.getByRole("button", { name: /^Restyle/ });
    fireEvent.click(restyle);
    fireEvent.click(restyle);
    await act(async () => maskExport.resolve(new Blob([new Uint8Array(4)])));
    await flush();
    await waitFor(() => expect(api.importImage).toHaveBeenCalledTimes(1));
    expect(screen.queryByText(/still working on the last image/)).toBeNull();
  });

  it("Side by side shows image 2 next to an edit that combined it, in place of Compare", async () => {
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    act(() => {
      store.dispatch({ type: "editLoad", ref: ref("a") });
      store.dispatch({ type: "editPush", ref: ref("e1") });
    });
    expect(screen.queryByRole("button", { name: /Side by side/ })).toBeNull();
    act(() => {
      store.dispatch({ type: "editSetSecond", ref: ref("b") });
      store.dispatch({ type: "editPush", ref: ref("e2"), secondImageId: "b" });
    });
    const side = await screen.findByRole("button", { name: /Side by side/ });
    const compare = screen.getByRole("button", { name: /Compare/ });
    fireEvent.click(compare);
    expect(compare.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(side);
    expect(side.getAttribute("aria-pressed")).toBe("true");
    expect(compare.getAttribute("aria-pressed")).toBe("false");
    expect(screen.getByTestId("side-by-side")).toBeTruthy();
    fireEvent.click(compare);
    expect(side.getAttribute("aria-pressed")).toBe("false");
    expect(screen.queryByTestId("side-by-side")).toBeNull();
  });

  it("with a second image, hides the brush and edits with a model that combines two images", async () => {
    const kontext = { ...model, id: "kx", familyId: "flux1_kontext", modes: ["edit"], isEditModel: true, fit: "fits" } as InstalledModel;
    const klein = { ...model, id: "kl", familyId: "flux2_klein_4b", modes: ["txt2img", "img2img", "edit"], multiRef: true, fit: "fits" } as InstalledModel;
    vi.mocked(api.listModels).mockImplementation(async () => [model, kontext, klein]);
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(3));
    act(() => {
      store.dispatch({ type: "editLoad", ref: ref("a") });
      store.dispatch({ type: "patchEdit", patch: { instruction: "put the bottle from image 2 on the shelf" } });
    });
    expect(await screen.findByRole("switch", { name: /Only change here/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Add another image/ })).toBeTruthy();

    act(() => store.dispatch({ type: "editSetSecond", ref: ref("b") }));
    expect(screen.queryByRole("switch", { name: /Only change here/ })).toBeNull();
    expect(screen.getByRole("button", { name: "Remove image 2" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /^Apply edit/ }));
    await waitFor(() => expect(api.generate).toHaveBeenCalledTimes(1));
    expect(vi.mocked(api.generate).mock.calls[0][0]).toMatchObject({ modelId: "kl", mode: "edit", refImageIds: ["a", "b"], maskImageId: null });

    act(() => store.dispatch({ type: "editSetSecond", ref: null }));
    expect(store.getState().edit.secondImageId).toBeNull();
    vi.mocked(api.listModels).mockImplementation(async () => [model]);
  });

  it("Improve adds details to the change with the edit model's family, and Undo puts it back", async () => {
    const kontext = { ...model, id: "kx", familyId: "flux1_kontext", modes: ["edit"], isEditModel: true, fit: "fits" } as InstalledModel;
    vi.mocked(api.listModels).mockImplementation(async () => [model, kontext]);
    captioner = { available: true, source: null, downloadBytes: 0, running: false } as CaptionerStatus;
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(2));
    act(() => {
      store.dispatch({ type: "editLoad", ref: ref("a") });
      store.dispatch({ type: "patchEdit", patch: { mode: "instruction", instruction: "make it evening" } });
    });
    // The family's settings load on their own: Improve sends the family once they're in.
    await waitFor(() => expect(store.getState().familyUi.flux1_kontext).toBeTruthy());
    fireEvent.click(await screen.findByRole("button", { name: /Improve/ }));
    await waitFor(() => expect(store.getState().edit.instruction).toBe("make it evening. Warm street lights. Keep the composition unchanged."));
    expect(api.improvePrompt).toHaveBeenCalledWith("make it evening", "flux1_kontext", [], "edit");
    expect(store.getState().create.prompt).toBe("");
    fireEvent.click(await screen.findByTitle("Put back what you wrote"));
    expect(store.getState().edit.instruction).toBe("make it evening");
    vi.mocked(api.listModels).mockImplementation(async () => [model]);
  });

  it("with a second image and no model that combines two, only offers one that can", async () => {
    const kontext = { ...model, id: "kx", familyId: "flux1_kontext", modes: ["edit"], isEditModel: true, fit: "fits" } as InstalledModel;
    vi.mocked(api.listModels).mockImplementation(async () => [model, kontext]);
    const pick = (familyId: string, title: string) =>
      ({ role: "edit", roleLabel: "Edit", title, familyId, goodAt: null, downloadBytes: 1, vram: null, fit: "fits", installed: false, quant: null, licenseNote: null }) as never;
    const recommended = vi.spyOn(api, "getRecommended").mockResolvedValue([pick("flux1_kontext", "Kontext card")]);
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(2));
    act(() => {
      store.dispatch({ type: "editLoad", ref: ref("a") });
      store.dispatch({ type: "editSetSecond", ref: ref("b") });
    });
    expect(await screen.findByText("Combining two images needs another edit model")).toBeTruthy();
    await flush();
    expect(screen.queryByText("Kontext card")).toBeNull();
    expect(screen.getByRole("button", { name: "Remove image 2" })).toBeTruthy();
    recommended.mockRestore();
    vi.mocked(api.listModels).mockImplementation(async () => [model]);
  });

  it("opened in Restyle without an edit model, offers the edit model in one line until closed", async () => {
    resetEditModelLine();
    const editPick = { role: "edit", roleLabel: "Edit", title: "Edit card", familyId: "qwen_image_edit_2511", goodAt: null, downloadBytes: 15e9, vram: null, fit: "fits", installed: false, quant: null, licenseNote: null, unavailableReason: null, note: null } as never;
    const recommended = vi.spyOn(api, "getRecommended").mockResolvedValue([editPick]);
    const install = vi.spyOn(api, "installRecommended").mockResolvedValue({ groupId: "g1" });
    const line = /To change one thing and keep the rest, get the edit model/;
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    const view = render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(1));
    expect((await screen.findByText(line)).textContent).toContain("15");
    fireEvent.click(screen.getByRole("button", { name: "Get Edit card" }));
    await waitFor(() => expect(install).toHaveBeenCalledWith("edit"));
    // Picking Restyle on purpose hides it; it's only for the automatic pick.
    act(() => store.dispatch({ type: "patchEdit", patch: { mode: "restyle" } }));
    expect(screen.queryByText(line)).toBeNull();
    act(() => store.dispatch({ type: "patchEdit", patch: { mode: null } }));
    fireEvent.click(await screen.findByRole("button", { name: "Close" }));
    expect(screen.queryByText(line)).toBeNull();
    // Closed for the rest of the session, also after the tab is shown again.
    view.unmount();
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await flush();
    expect(screen.queryByText(line)).toBeNull();
    recommended.mockRestore();
    install.mockRestore();
    resetEditModelLine();
  });

  it("doesn't offer the edit model once one is installed", async () => {
    resetEditModelLine();
    const recommended = vi.spyOn(api, "getRecommended").mockResolvedValue([
      { role: "edit", roleLabel: "Edit", title: "Edit card", familyId: "x", goodAt: null, downloadBytes: 1, vram: null, fit: "fits", installed: false, quant: null, licenseNote: null, unavailableReason: null, note: null } as never,
    ]);
    const edit = { ...model, id: "qe", familyId: "qwen_image_edit_2511", modes: ["edit"], isEditModel: true, fit: "fits" } as InstalledModel;
    vi.mocked(api.listModels).mockImplementation(async () => [model, edit]);
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(2));
    await flush();
    expect(screen.getByText("Picked automatically — you have an edit model.")).toBeTruthy();
    expect(screen.queryByText(/get the edit model/)).toBeNull();
    recommended.mockRestore();
    vi.mocked(api.listModels).mockImplementation(async () => [model]);
  });

  it("shows the Edit tab's own add-on chips", async () => {
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(1));
    act(() => {
      store.dispatch({ type: "editLoad", ref: ref("a") });
      store.dispatch({ type: "patchEdit", patch: { mode: "restyle", loras: [{ loraId: "gone", weight: 0.8 }] } });
    });
    fireEvent.click(await screen.findByRole("button", { name: "Remove Missing add-on" }));
    expect(store.getState().edit.loras).toEqual([]);
    expect(store.getState().create.loras).toEqual([]);
  });

  it("Try again redoes the shown edit from the step before; the original can't be tried again", async () => {
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(1));
    act(() => {
      store.dispatch({ type: "editLoad", ref: ref("a") });
      store.dispatch({ type: "patchEdit", patch: { mode: "restyle", restylePrompt: "watercolor", seed: 5 } });
    });
    const tryAgain = await screen.findByRole("button", { name: "Try again" });
    expect(tryAgain).toHaveProperty("disabled", true);
    const meta = { id: "r", width: 64, height: 64, seed: 5, modelId: "m", modelLabel: "Test model", familyId: "", steps: 1, cfg: 1, guidance: null, sampler: null, scheduler: null, parentId: "a" };
    act(() => store.dispatch({ type: "editPush", ref: ref("r"), meta }));
    await flush();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(api.generate).toHaveBeenCalledTimes(1));
    const req = vi.mocked(api.generate).mock.calls[0][0];
    expect(req.initImageId).toBe("a");
    expect(req.fineTune.seed).toBeUndefined();
  });

  it("Try again stays available after a second Add detail in a row", async () => {
    paintOnShow = false;
    const results: ((r: unknown) => void)[] = [];
    vi.mocked(api.generate).mockImplementation(() => new Promise((res) => results.push(res as (r: unknown) => void)) as never);
    const getImage = vi.spyOn(api, "getImage").mockResolvedValue(new ArrayBuffer(8));
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(1));
    act(() => store.dispatch({ type: "editLoad", ref: ref("a") }));
    fireEvent.click(await screen.findByRole("radio", { name: "Fix details" }));
    await flush();
    const meta = (id: string, parentId: string) => ({ id, width: 64, height: 64, seed: 1, modelId: "m", modelLabel: "Test model", familyId: "", steps: 1, cfg: 1, guidance: null, sampler: null, scheduler: null, parentId });
    for (const [n, [id, parent]] of [["r1", "a"], ["r2", "r1"]].entries()) {
      fireEvent.click(screen.getByRole("button", { name: /^Add detail/ }));
      await waitFor(() => expect(results).toHaveLength(n + 1));
      await act(async () => results[n]({ images: [meta(id, parent)] }));
      await waitFor(() => expect(store.getState().job).toBeNull());
      await flush();
      expect(store.getState().edit.chain.at(-1)?.imageId).toBe(id);
      expect(screen.getByRole("button", { name: "Try again" })).toHaveProperty("disabled", false);
    }
    getImage.mockRestore();
    vi.mocked(api.generate).mockImplementation(() => new Promise(() => undefined));
  });

  it("adds an edit queued behind another after its result instead of replacing it", async () => {
    const results: ((r: unknown) => void)[] = [];
    vi.mocked(api.generate).mockImplementation(() => new Promise((res) => results.push(res as (r: unknown) => void)) as never);
    const getImage = vi.spyOn(api, "getImage").mockResolvedValue(new ArrayBuffer(8));
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(1));
    act(() => {
      store.dispatch({ type: "editLoad", ref: ref("a") });
      store.dispatch({ type: "patchEdit", patch: { mode: "restyle", restylePrompt: "watercolor" } });
    });
    await flush();
    act(() => void runPrimaryAction("edit"));
    await waitFor(() => expect(results).toHaveLength(1));
    act(() => store.dispatch({ type: "patchEdit", patch: { restylePrompt: "oil paint" } }));
    await flush();
    act(() => void runPrimaryAction("edit"));
    await waitFor(() => expect(store.getState().queue).toHaveLength(1));
    const meta = (id: string) => ({ id, width: 64, height: 64, seed: 1, modelId: "m", modelLabel: "Test model", familyId: "", steps: 1, cfg: 1, guidance: null, sampler: null, scheduler: null, parentId: "a" });
    await act(async () => results[0]({ images: [meta("r1")] }));
    await waitFor(() => expect(results).toHaveLength(2));
    await act(async () => results[1]({ images: [meta("r2")] }));
    await waitFor(() => expect(store.getState().job).toBeNull());
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["a", "r1", "r2"]);
    getImage.mockRestore();
    vi.mocked(api.generate).mockImplementation(() => new Promise(() => undefined));
  });

  it("Ctrl+Z doesn't move the history behind an open dialog", async () => {
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
        <div role="dialog">
          <button type="button">In the dialog</button>
        </div>
      </AppProvider>,
    );
    const meta = { id: "r", width: 64, height: 64, seed: 1, modelId: "m", modelLabel: "Test model", familyId: "", steps: 1, cfg: 1, guidance: null, sampler: null, scheduler: null, parentId: "a" };
    act(() => {
      store.dispatch({ type: "editLoad", ref: ref("a") });
      store.dispatch({ type: "editPush", ref: ref("r"), meta });
    });
    await flush();
    fireEvent.keyDown(screen.getByRole("button", { name: "In the dialog" }), { key: "z", ctrlKey: true });
    expect(store.getState().edit.index).toBe(1);
    cleanup();
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await flush();
    fireEvent.keyDown(window, { key: "z", ctrlKey: true });
    expect(store.getState().edit.index).toBe(0);
  });

  it("draws the toolbar's undo, redo and delete buttons at the bar's small size, and the full-screen button over the picture", async () => {
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    act(() => {
      store.dispatch({ type: "editLoad", ref: ref("a") });
      store.dispatch({ type: "patchEdit", patch: { mode: "restyle" } });
    });
    for (const name of ["Undo", "Redo", "Delete this edit"]) {
      expect((await screen.findByRole("button", { name })).className.split(" ")).toContain("h-7");
    }
    const full = await screen.findByRole("button", { name: "View full screen" });
    expect(full.className.split(" ")).toEqual(expect.arrayContaining(["bg-black/40!", "text-white!", "hover:bg-black/60!"]));
  });

  it("offers Upscale and shows the final prompt in Fine-tune", async () => {
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(1));
    act(() => {
      store.dispatch({ type: "editLoad", ref: ref("a") });
      store.dispatch({ type: "patchEdit", patch: { mode: "restyle", restylePrompt: "watercolor lighthouse" } });
    });
    fireEvent.click(await screen.findByRole("button", { name: /^Upscale/ }));
    expect(await screen.findByText("Upscale 2×")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Fine-tune/ }));
    expect(await screen.findByText("Final prompt sent to the model")).toBeTruthy();
    await waitFor(() => expect(screen.getByText(/watercolor lighthouse/, { selector: "div" })).toBeTruthy(), { timeout: 2000 });
  });
});

describe("Compare slider", () => {
  it("stops following the pointer after a pointercancel", () => {
    render(<CompareView before={ref("b")} after={ref("a")} width={100} height={100} />);
    const slider = screen.getByRole("slider");
    const box = slider.parentElement!;
    box.getBoundingClientRect = () => ({ left: 0, top: 0, right: 100, bottom: 100, width: 100, height: 100, x: 0, y: 0, toJSON: () => ({}) });
    box.setPointerCapture = () => undefined;
    fireEvent.pointerDown(box, { clientX: 20, buttons: 1, pointerId: 1 });
    expect(slider.getAttribute("aria-valuenow")).toBe("20");
    fireEvent.pointerCancel(box, { pointerId: 1 });
    fireEvent.pointerMove(box, { clientX: 80, buttons: 1, pointerId: 1 });
    expect(slider.getAttribute("aria-valuenow")).toBe("20");
  });
});

describe("Describe tab", () => {
  it("checks the describer only while the tab is open", async () => {
    const store = createStore();
    render(
      <AppProvider store={store}>
        <DescribeTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(1));
    await flush();
    expect(api.captionerStatus).not.toHaveBeenCalled();
    act(() => store.dispatch({ type: "setModels", models: [model] }));
    await flush();
    expect(api.captionerStatus).not.toHaveBeenCalled();
    act(() => store.dispatch({ type: "setTab", tab: "describe" }));
    await waitFor(() => expect(api.captionerStatus).toHaveBeenCalledTimes(1));
  });

  it("Ctrl+Enter doesn't describe while the describer isn't installed", async () => {
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "describe" });
    store.dispatch({ type: "describeLoad", ref: ref("a") });
    render(
      <AppProvider store={store}>
        <DescribeTab />
      </AppProvider>,
    );
    await waitFor(() => expect(api.captionerStatus).toHaveBeenCalled());
    await flush();
    act(() => void runPrimaryAction("describe"));
    await flush();
    expect(api.describeImage).not.toHaveBeenCalled();
  });
});

describe("pictures from this session", () => {
  it("Edit: the empty tab and Another image offer this session's pictures", async () => {
    const store = withResults("a", "b");
    store.dispatch({ type: "setTab", tab: "edit" });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await flush();
    expect(screen.getByText("Or use one from this session")).toBeTruthy();
    expect(sessionThumbs("Edit this picture")).toEqual(["blob:a", "blob:b"]);
    fireEvent.click(screen.getAllByRole("button", { name: "Edit this picture" })[1]);
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["b"]);

    // Another image: a file, or one of the others (not the one shown).
    fireEvent.click(screen.getByRole("button", { name: "Another image" }));
    expect(screen.getByRole("menuitem", { name: /Choose a file/ })).toBeTruthy();
    expect(sessionThumbs("Edit this picture")).toEqual(["blob:a"]);
    fireEvent.click(screen.getByRole("button", { name: "Edit this picture" }));
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["a"]);
    expect(api.importImage).not.toHaveBeenCalled();
  });

  it("Edit: Add another image can use a picture from this session as image 2", async () => {
    const klein = { ...model, id: "kl", familyId: "flux2_klein_4b", modes: ["txt2img", "img2img", "edit"], multiRef: true, fit: "fits" } as InstalledModel;
    vi.mocked(api.listModels).mockImplementation(async () => [model, klein]);
    const store = withResults("a", "b");
    store.dispatch({ type: "setTab", tab: "edit" });
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "patchEdit", patch: { mode: "instruction" } });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models?.length).toBe(2));
    fireEvent.click(screen.getByRole("button", { name: /Add another image/ }));
    expect(sessionThumbs("Use as image 2")).toEqual(["blob:b"]);
    fireEvent.click(screen.getByRole("button", { name: "Use as image 2" }));
    expect(store.getState().edit.secondImageId).toBe("b");
    expect(screen.getByRole("button", { name: "Remove image 2" })).toBeTruthy();
    // Picking image 2 as the picture to edit leaves no image 2 behind (it can't be both).
    fireEvent.click(screen.getByRole("button", { name: "Another image" }));
    fireEvent.click(screen.getByRole("button", { name: "Edit this picture" }));
    expect(store.getState().edit.chain.map((n) => n.imageId)).toEqual(["b"]);
    expect(store.getState().edit.secondImageId).toBeNull();
    vi.mocked(api.listModels).mockImplementation(async () => [model]);
  });

  it("Edit: with nothing made this session, Another image opens the file chooser", async () => {
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    store.dispatch({ type: "editLoad", ref: ref("photo") });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await flush();
    expect(screen.queryByText("Or use one from this session")).toBeNull();
    const click = vi.spyOn(HTMLInputElement.prototype, "click").mockImplementation(() => undefined);
    fireEvent.click(screen.getByRole("button", { name: "Another image" }));
    expect(click).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("menuitem", { name: /Choose a file/ })).toBeNull();
    click.mockRestore();
  });

  it("Describe: the empty tab and Another image offer this session's pictures", async () => {
    const store = withResults("a", "b");
    store.dispatch({ type: "setTab", tab: "describe" });
    render(
      <AppProvider store={store}>
        <DescribeTab />
      </AppProvider>,
    );
    await flush();
    fireEvent.click(screen.getAllByRole("button", { name: "Describe this picture" })[0]);
    expect(store.getState().describe.imageId).toBe("a");
    fireEvent.click(screen.getByRole("button", { name: /Another image/ }));
    expect(sessionThumbs("Describe this picture")).toEqual(["blob:b"]);
    fireEvent.click(screen.getByRole("button", { name: "Describe this picture" }));
    expect(store.getState().describe.imageId).toBe("b");
  });
});

describe("Also apply to…", () => {
  it("picks pictures from this session, counts them on the button and hides the brush", async () => {
    const store = createStore();
    store.dispatch({ type: "setTab", tab: "edit" });
    store.dispatch({
      type: "addResults",
      batch: null,
      images: [{ id: "c1", width: 64, height: 64, seed: 1, modelId: "m", modelLabel: "M", familyId: "sdxl", steps: 1, cfg: 1, guidance: null, sampler: null, scheduler: null, parentId: null, origin: "generated" }],
      refs: [ref("c1")],
    });
    store.dispatch({ type: "editLoad", ref: ref("a") });
    store.dispatch({ type: "patchEdit", patch: { mode: "restyle", restylePrompt: "watercolor" } });
    render(
      <AppProvider store={store}>
        <EditTab />
      </AppProvider>,
    );
    await flush();
    expect(screen.getByText("Only change here")).toBeTruthy();
    fireEvent.click(screen.getByRole("switch", { name: /Only change here/ }));
    await flush();
    expect(maskActive).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: /Also apply to/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Also apply to this picture" }));
    await flush();
    expect(store.getState().edit.alsoIds).toEqual(["c1"]);
    expect(screen.getByRole("button", { name: /Restyle 2 pictures/ })).toBeTruthy();
    expect(screen.queryByText("Only change here")).toBeNull();
    // The brush is off too: the edit changes each whole picture.
    expect(maskActive).toBe(false);
    expect(screen.getByText(/Also on 1 more picture/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Don't apply to other pictures" }));
    expect(store.getState().edit.alsoIds).toEqual([]);
  });
});

