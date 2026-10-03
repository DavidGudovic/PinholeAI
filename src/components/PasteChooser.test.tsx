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
    readPictureSettings: vi.fn(async () => null),
  };
});

const api = await import("../lib/api");
const { installMocks } = await import("../lib/mock");
const { AppProvider } = await import("../lib/state/AppProvider");
const { createStore } = await import("../lib/state/store");
const { PasteChooser, defaultPasteTarget, pasteTargets } = await import("./PasteChooser");
const { DropTarget, offerDroppedPicture } = await import("./ImageDrop");
const { Toasts } = await import("./Toasts");
const { CreateTab } = await import("../tabs/create/CreateTab");
const { blockStrayDrops } = await import("../lib/platform");

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

/** jsdom has no DragEvent/DataTransfer: a plain cancelable event carrying a fake transfer. */
function drop(target: Element, data: { types: string[]; files?: File[]; uri?: string; html?: string }) {
  const e = new Event("drop", { bubbles: true, cancelable: true });
  Object.defineProperty(e, "dataTransfer", {
    value: { types: data.types, files: data.files ?? [], items: [], dropEffect: "copy", getData: (t: string) => (t === "text/uri-list" ? (data.uri ?? "") : t === "text/html" ? (data.html ?? "") : "") },
  });
  act(() => {
    target.dispatchEvent(e);
  });
  return e;
}
const picture = () => new File([new Uint8Array(4)], "photo.jpg", { type: "image/jpeg" });

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

describe("dropped picture", () => {
  let stop: () => void;
  beforeEach(() => {
    stop = blockStrayDrops();
  });
  afterEach(() => stop());

  it("dropped where no drop area takes it, asks what it's for like a pasted one", async () => {
    const store = setup(["txt2img"]);
    const e = drop(document.body, { types: ["Files"], files: [picture()] });
    expect(e.defaultPrevented).toBe(true);
    expect(screen.getByRole("dialog", { name: /Use the dropped picture for/ })).toBeTruthy();
    expect(api.importImage).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: /^Edit/ }));
    await flush();
    expect(store.getState().tab).toBe("edit");
    expect(api.importImage).toHaveBeenCalledTimes(1);
  });

  it("a drop area that takes the picture keeps it", () => {
    setup(["txt2img"]);
    const onFile = vi.fn();
    render(
      <DropTarget onFile={onFile}>
        <span>Reference slot</span>
      </DropTarget>,
    );
    const f = picture();
    drop(screen.getByText("Reference slot"), { types: ["Files"], files: [f] });
    expect(onFile).toHaveBeenCalledWith(f);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("says what to do with a file that isn't a picture or a link from a web browser", () => {
    const store = createStore();
    render(
      <AppProvider store={store}>
        <PasteChooser />
        <Toasts />
      </AppProvider>,
    );
    // The window takes a link while it's dragged over (else the drop never comes).
    const over = new Event("dragover", { bubbles: true, cancelable: true });
    Object.defineProperty(over, "dataTransfer", { value: { types: ["text/uri-list"], dropEffect: "none" } });
    document.body.dispatchEvent(over);
    expect((over as Event & { dataTransfer: { dropEffect: string } }).dataTransfer.dropEffect).toBe("copy");
    drop(document.body, { types: ["Files"], files: [new File(["x"], "notes.txt", { type: "text/plain" })] });
    expect(screen.getByText(/isn’t a picture Pinhole can open/)).toBeTruthy();
    const e = drop(document.body, { types: ["text/uri-list"], uri: "https://example.com/cat.png" });
    expect(e.defaultPrevented).toBe(true);
    expect(screen.getByText(/choose Copy image, then paste it here/)).toBeTruthy();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("dropped on Create's results without saved settings, asks what it's for", async () => {
    const store = createStore();
    store.dispatch({ type: "setModels", models: [model("m", ["txt2img"])] });
    store.dispatch({ type: "selectModel", modelId: "m" });
    render(
      <AppProvider store={store}>
        <CreateTab />
        <PasteChooser />
      </AppProvider>,
    );
    drop(screen.getByText("Your images appear here"), { types: ["Files"], files: [picture()] });
    await flush();
    expect(api.readPictureSettings).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("dialog", { name: /Use the dropped picture for/ })).toBeTruthy();
  });

  it("is not offered while another dialog is open", () => {
    setup(["txt2img"]);
    render(<div role="dialog" aria-label="Viewer" />);
    act(() => offerDroppedPicture(picture()));
    expect(screen.queryByRole("dialog", { name: /Use the dropped picture for/ })).toBeNull();
  });

  it("a browser drag holding the picture in a data: address opens the chooser, with nothing downloaded", () => {
    setup(["txt2img"]);
    const png = "data:image/png;base64,iVBORw0KGgo=";
    // With a web address, the markup's src may be a placeholder: not used.
    drop(document.body, { types: ["text/uri-list", "text/html"], uri: "https://cdn.example.com/real.jpg", html: `<img src="${png}" srcset="https://cdn.example.com/real.jpg">` });
    expect(screen.queryByRole("dialog")).toBeNull();
    drop(document.body, { types: ["text/uri-list", "text/html"], uri: "", html: `<meta charset="utf-8"><img alt="" data-src="data:image/png;base64,AAAA" src="${png}">` });
    expect(screen.getByRole("dialog", { name: /Use the dropped picture for/ })).toBeTruthy();
  });
});
