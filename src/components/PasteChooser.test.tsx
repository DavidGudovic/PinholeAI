// @vitest-environment jsdom
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { InstalledModel } from "../lib/types";

vi.mock("../lib/api", async (orig) => {
  const real = await orig<typeof import("../lib/api")>();
  let n = 0;
  return {
    ...real,
    importImage: vi.fn(async () => ({ id: `pasted-${++n}`, width: 64, height: 48 })),
    getImage: vi.fn(async () => new Uint8Array([0x89, 0x50, 0x4e, 0x47]).buffer),
  };
});

const api = await import("../lib/api");
const { installMocks } = await import("../lib/mock");
const { AppProvider } = await import("../lib/state/AppProvider");
const { createStore } = await import("../lib/state/store");
const { PasteChooser, defaultPasteTarget, pasteTargets } = await import("./PasteChooser");

beforeAll(async () => {
  await installMocks();
});
beforeEach(() => {
  globalThis.URL.createObjectURL = vi.fn(() => "blob:x") as typeof URL.createObjectURL;
  globalThis.URL.revokeObjectURL = vi.fn();
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const model = (id: string, modes: string[]) =>
  ({ id, friendlyName: id, familyId: null, modes, isEditModel: false, fit: null, missingComponents: [], lastUsed: null }) as unknown as InstalledModel;

function paste() {
  const file = new File([new Uint8Array(4)], "image.png", { type: "image/png" });
  const e = new Event("paste", { bubbles: true, cancelable: true }) as Event & { clipboardData: unknown };
  Object.defineProperty(e, "clipboardData", { value: { types: ["Files"], files: [file], items: [] } });
  act(() => {
    document.body.dispatchEvent(e);
  });
}
const flush = () => act(() => new Promise((r) => setTimeout(r, 0)));

function setup(createModes: string[]) {
  const store = createStore();
  store.dispatch({ type: "setModels", models: [model("m", createModes)] });
  store.dispatch({ type: "selectModel", modelId: "m" });
  render(
    <AppProvider store={store}>
      <PasteChooser />
    </AppProvider>,
  );
  return store;
}

describe("pasted picture", () => {
  it("offers Edit and Describe, and the Create reference only for a model that takes one", () => {
    expect(pasteTargets(false)).toEqual(["edit", "describe"]);
    expect(pasteTargets(true)).toEqual(["reference", "edit", "describe"]);
    setup(["txt2img"]);
    paste();
    expect(screen.getByRole("dialog", { name: /Use the pasted picture for/ })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Create reference picture/ })).toBeNull();
    expect(screen.getByRole("button", { name: /^Edit/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /^Describe/ })).toBeTruthy();
    expect(api.importImage).not.toHaveBeenCalled();
  });

  it("puts the current tab's choice first", () => {
    const t = pasteTargets(true);
    expect(defaultPasteTarget("create", t)).toBe("reference");
    expect(defaultPasteTarget("create", pasteTargets(false))).toBe("edit");
    expect(defaultPasteTarget("describe", t)).toBe("describe");
    expect(defaultPasteTarget("models", t)).toBe("edit");
  });

  it("imports into Describe and opens it", async () => {
    const store = setup(["txt2img"]);
    paste();
    fireEvent.click(screen.getByRole("button", { name: /^Describe/ }));
    await flush();
    expect(api.importImage).toHaveBeenCalledTimes(1);
    const s = store.getState();
    expect(s.tab).toBe("describe");
    expect(s.describe.imageId).toBe("pasted-1");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("imports into Edit and opens it", async () => {
    const store = setup(["txt2img"]);
    paste();
    fireEvent.click(screen.getByRole("button", { name: /^Edit/ }));
    await flush();
    const s = store.getState();
    expect(s.tab).toBe("edit");
    expect(s.edit.chain.map((n) => n.imageId)).toEqual([expect.stringMatching(/^pasted-/)]);
  });

  it("sets the Create reference picture", async () => {
    const store = setup(["txt2img", "edit"]);
    store.dispatch({ type: "setTab", tab: "models" });
    paste();
    fireEvent.click(screen.getByRole("button", { name: /Create reference picture/ }));
    await flush();
    const s = store.getState();
    expect(s.tab).toBe("create");
    expect(s.create.refImageId).toMatch(/^pasted-/);
  });

  it("closes without importing", () => {
    setup(["txt2img"]);
    paste();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(api.importImage).not.toHaveBeenCalled();
  });
});
