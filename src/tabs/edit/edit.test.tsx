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
  };
});

// A canvas-free mask: painted as soon as it is switched on; its export waits for the test.
vi.mock("./MaskCanvas", () => ({
  MaskCanvas: ({ ref, active, onPaintedChange }: { ref?: Ref<unknown>; active: boolean; onPaintedChange: (b: boolean) => void }) => {
    useImperativeHandle(ref, () => ({ clear: () => undefined, exportPng: () => maskExport.promise }));
    useEffect(() => {
      if (active) onPaintedChange(true);
    }, [active, onPaintedChange]);
    return null;
  },
}));

const api = await import("../../lib/api");
const { installMocks } = await import("../../lib/mock");
const { AppProvider, runPrimaryAction } = await import("../../lib/state/AppProvider");
const { createStore } = await import("../../lib/state/store");
const { EditTab } = await import("./EditTab");
const { CompareView } = await import("./CompareView");
const { DescribeTab } = await import("../describe/DescribeTab");

beforeAll(async () => {
  await installMocks();
});
beforeEach(() => {
  maskExport = hold();
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

describe("Edit tab", () => {
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
