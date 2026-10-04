// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { Style } from "../lib/types";

let finish!: (s: Style) => void;
vi.mock("../lib/api", async (orig) => ({
  ...(await orig<typeof import("../lib/api")>()),
  saveStyle: vi.fn(() => new Promise<Style>((r) => (finish = r))),
}));

const api = await import("../lib/api");
const { installMocks } = await import("../lib/mock");
const { AppProvider } = await import("../lib/state/AppProvider");
const { createStore } = await import("../lib/state/store");
const { StyleEditorDialog } = await import("./StylePicker");

beforeAll(async () => {
  await installMocks();
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("Save as style", () => {
  it("Enter in the name field saves once, even when pressed twice", async () => {
    const onClose = vi.fn();
    render(
      <AppProvider store={createStore()}>
        <StyleEditorDialog open onClose={onClose} initial={null} familyId={null} familyLabel={null} />
      </AppProvider>,
    );
    fireEvent.change(screen.getByPlaceholderText("Words that describe the look"), { target: { value: "soft light" } });
    const name = screen.getByPlaceholderText("e.g. Soft film look");
    fireEvent.change(name, { target: { value: "Soft" } });
    fireEvent.submit(name.closest("form")!);
    fireEvent.submit(name.closest("form")!);
    expect(api.saveStyle).toHaveBeenCalledTimes(1);
    await act(async () => finish({ id: "s1", name: "Soft", positive: "soft light", negative: null, families: [], thumbnail: null, builtin: false } as Style));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });
});
