// @vitest-environment jsdom
import { beforeAll, describe, expect, it, vi } from "vitest";
import type { InstalledModel, PictureSettings } from "../../lib/types";

vi.mock("../../lib/api", async (orig) => ({ ...(await orig<typeof import("../../lib/api")>()), readPictureSettings: vi.fn() }));

const api = await import("../../lib/api");
const { installMocks } = await import("../../lib/mock");
const { createStore } = await import("../../lib/state/store");
const { makeActions } = await import("../../lib/state/actions");
const { reuseSettingsFrom } = await import("./reuseSettings");

beforeAll(async () => {
  await installMocks();
});

const model = (id: string, friendlyName: string) =>
  ({ id, friendlyName, familyId: null, familyLabel: "Test", modes: ["txt2img"], isEditModel: false, missingComponents: [] }) as unknown as InstalledModel;

function setup(settings: PictureSettings | null) {
  vi.mocked(api.readPictureSettings).mockResolvedValue(settings);
  const store = createStore();
  store.dispatch({ type: "setModels", models: [model("a", "Alpha"), model("b", "Beta")] });
  store.dispatch({ type: "selectModel", modelId: "a" });
  store.dispatch({ type: "patchCreate", patch: { prompt: "my own prompt", loras: [{ loraId: "l1", weight: 0.7 }] } });
  return { store, actions: makeActions(store) };
}
const blob = () => new Blob([new Uint8Array([1, 2, 3])]);

describe("reuse settings from a saved picture", () => {
  it("selects the model it was made with and applies its settings, keeping prompt and add-ons", async () => {
    const { store, actions } = setup({ modelId: "b", model: "Beta", seed: 99, steps: 28, cfg: 6.5, sampler: "euler_a", scheduler: "karras", width: 832, height: 1216 });
    const o = await reuseSettingsFrom(blob(), store, actions);
    const c = store.getState().create;
    expect(o).toMatchObject({ found: true, modelSelected: true, modelName: "Beta" });
    expect(c.modelId).toBe("b");
    expect(c.fineTune).toMatchObject({ seed: 99, steps: 28, sampler: "euler_a", scheduler: "karras" });
    expect(c.shape).toBe("portrait");
    expect(c.prompt).toBe("my own prompt");
    expect(c.loras).toEqual([{ loraId: "l1", weight: 0.7 }]);
  });

  it("finds an older picture's model by name and keeps the current one when it isn't installed", async () => {
    let { store, actions } = setup({ model: "Beta", steps: 10 });
    await reuseSettingsFrom(blob(), store, actions);
    expect(store.getState().create.modelId).toBe("b");

    ({ store, actions } = setup({ modelId: "gone", model: "Gone model", steps: 10 }));
    const o = await reuseSettingsFrom(blob(), store, actions);
    expect(o).toMatchObject({ found: true, modelSelected: false, madeWith: "Gone model", modelName: "Alpha" });
    expect(store.getState().create.modelId).toBe("a");
    expect(store.getState().create.fineTune.steps).toBe(10);
  });

  it("changes nothing for a picture without settings", async () => {
    const { store, actions } = setup(null);
    const before = store.getState().create;
    const o = await reuseSettingsFrom(blob(), store, actions);
    expect(o.found).toBe(false);
    expect(store.getState().create).toBe(before);
  });

  it("turns Repeats without seams on when the picture was made with it, off otherwise", async () => {
    let { store, actions } = setup({ steps: 20, seamless: true });
    await reuseSettingsFrom(blob(), store, actions);
    expect(store.getState().create.fineTune.seamless).toBe(true);

    vi.mocked(api.readPictureSettings).mockResolvedValue({ steps: 20, seamless: false });
    await reuseSettingsFrom(blob(), store, actions);
    expect(store.getState().create.fineTune.seamless ?? null).toBeNull();

    // Pictures saved before it was recorded: off, like the other settings they don't carry.
    ({ store, actions } = setup({ steps: 20 }));
    store.dispatch({ type: "setFineTune", patch: { seamless: true } });
    await reuseSettingsFrom(blob(), store, actions);
    expect(store.getState().create.fineTune.seamless ?? null).toBeNull();
  });

  it("ignores a sampler name the engine doesn't have", async () => {
    const { store, actions } = setup({ steps: 12, sampler: "made-up" });
    await reuseSettingsFrom(blob(), store, actions);
    expect(store.getState().create.fineTune.sampler ?? null).toBeNull();
  });
});
