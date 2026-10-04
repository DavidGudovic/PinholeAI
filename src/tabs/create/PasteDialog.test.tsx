// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";

const DATA = "a ship\nNegative prompt: blurry\nSteps: 30, Sampler: Euler a, CFG scale: 7, Seed: 5, Size: 832x1216";
let clipboard: (text: string) => void = () => undefined;
vi.mock("../../lib/state/platform", async (orig) => {
  const real = await orig<typeof import("../../lib/state/platform")>();
  return { ...real, readClipboardText: () => new Promise<string>((res) => (clipboard = res)) };
});

const { installMocks } = await import("../../lib/mock");
const { AppProvider } = await import("../../lib/state/AppProvider");
const { createStore } = await import("../../lib/state/store");
const { PasteDialog } = await import("./PasteDialog");

beforeAll(async () => {
  await installMocks();
});
afterEach(cleanup);

function open() {
  render(
    <AppProvider store={createStore()}>
      <PasteDialog open onClose={() => undefined} onApply={async () => undefined} />
    </AppProvider>,
  );
  return screen.getByLabelText("Generation data") as HTMLTextAreaElement;
}

describe("Paste dialog", () => {
  it("fills in generation data from the clipboard", async () => {
    const box = open();
    await act(async () => clipboard(DATA));
    expect(box.value).toBe(DATA);
    expect(screen.getByText("Filled in from your clipboard.")).toBeTruthy();
  });

  it("keeps text typed before the clipboard was read, without the clipboard note", async () => {
    const box = open();
    fireEvent.change(box, { target: { value: "a castle" } });
    await act(async () => clipboard(DATA));
    expect(box.value).toBe("a castle");
    expect(screen.queryByText("Filled in from your clipboard.")).toBeNull();
  });
});
