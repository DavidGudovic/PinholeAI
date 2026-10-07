// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ResultImage } from "../lib/types";

let finishSave: (() => void) | null = null;
vi.mock("../lib/api", async (orig) => {
  const real = await orig<typeof import("../lib/api")>();
  return {
    ...real,
    saveImagesTo: vi.fn(
      (ids: string[], dir: string) =>
        new Promise((res) => (finishSave = () => res({ saved: ids.map((id) => ({ id, path: `${dir}/${id}.png` })), failed: 0 }))),
    ),
  };
});
const closeWindow = vi.fn(async () => undefined);
vi.mock("../lib/state/platform", async (orig) => {
  const real = await orig<typeof import("../lib/state/platform")>();
  return { ...real, canSaveAs: () => true, chooseFolder: async () => "/x", closeWindow: () => closeWindow() };
});

const { installMocks } = await import("../lib/mock");
const { AppProvider } = await import("../lib/state/AppProvider");
const { createStore } = await import("../lib/state/store");
const { UnsavedDialog } = await import("./UnsavedDialog");

beforeAll(async () => {
  await installMocks();
});
afterEach(cleanup);

function open(what: "close" | "clear" | "edit", before?: (store: ReturnType<typeof createStore>) => void) {
  const store = createStore();
  before?.(store);
  render(
    <AppProvider store={store}>
      <UnsavedDialog />
    </AppProvider>,
  );
  act(() => store.dispatch({ type: "askLeave", what }));
  return store;
}

const img = (id: string) => ({ id, width: 64, height: 64, seed: 1 }) as unknown as ResultImage;
const ref = (id: string) => ({ id, url: `blob:${id}`, width: 64, height: 64 });

describe("UnsavedDialog", () => {
  it("warns in red that Reset deletes unsaved images, prompts and Fine-tune changes", () => {
    open("clear");
    const warning = screen.getByText("Unsaved images, prompts and Fine-tune changes are permanently deleted.");
    expect(warning.className).toContain("text-red-700");
    expect(warning.className).toContain("dark:text-red-400");
    expect(screen.getByText("Nothing is saved until you press Save.")).toBeTruthy();
    expect(screen.queryByText(/for good/)).toBeNull();
  });

  it("says the same when closing, and names only edited images when editing another image", () => {
    open("close");
    expect(screen.getByText("Unsaved images, prompts and Fine-tune changes are permanently deleted.")).toBeTruthy();
    cleanup();
    open("edit");
    expect(screen.getByText("Unsaved edited images are permanently deleted.")).toBeTruthy();
  });

  it("focuses the button that goes ahead, so Enter closes the window", async () => {
    open("close");
    await act(() => new Promise((r) => setTimeout(r, 0)));
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Close without saving" }));
  });

  it("stays open after Save all when another picture finished during the save", async () => {
    const store = open("close", (s) => s.dispatch({ type: "addResults", batch: null, images: [img("a")], refs: [ref("a")] }));
    fireEvent.click(screen.getByRole("button", { name: /Save all/ }));
    await vi.waitFor(() => expect(finishSave).not.toBeNull());
    act(() => store.dispatch({ type: "addResults", batch: null, images: [img("b")], refs: [ref("b")] }));
    await act(async () => finishSave!());
    expect(closeWindow).not.toHaveBeenCalled();
    expect(store.getState().leave).toBe("close");
    expect(screen.getByText("You have 1 picture that isn't saved")).toBeTruthy();
  });
});
