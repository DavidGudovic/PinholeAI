// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, waitFor } from "@testing-library/react";

vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, listModels: vi.fn(real.listModels) };
});

const api = await import("../api");
const { installMocks, mockEmit } = await import("../mock");
const { AppProvider } = await import("./AppProvider");
const { createStore } = await import("./store");

beforeAll(async () => {
  await installMocks();
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const flush = () => act(() => new Promise((r) => setTimeout(r, 0)));
const calls = () => vi.mocked(api.listModels).mock.calls.length;

async function mounted() {
  const store = createStore();
  render(<AppProvider store={store}>{null}</AppProvider>);
  await waitFor(() => expect(store.getState().models).not.toBeNull());
  await waitFor(() => expect(store.getState().settings).not.toBeNull());
  await flush();
  return store;
}

describe("AppProvider model list", () => {
  it("fetches the models again when hardware detection finishes", async () => {
    await mounted();
    const before = calls();
    mockEmit("hardware-ready", null);
    await waitFor(() => expect(calls()).toBe(before + 1));
  });

  it("fetches the models again when a GPU, VRAM or backend setting changes, not for other settings", async () => {
    const store = await mounted();
    const before = calls();
    const settings = store.getState().settings!;

    act(() => store.dispatch({ type: "setSettings", settings: { ...settings, theme: settings.theme === "dark" ? "light" : "dark" } }));
    await flush();
    expect(calls()).toBe(before);

    act(() => store.dispatch({ type: "setSettings", settings: { ...store.getState().settings!, vramOverrideGb: 24 } }));
    await flush();
    expect(calls()).toBe(before + 1);

    act(() => store.dispatch({ type: "setSettings", settings: { ...store.getState().settings!, engineBackend: "cpu" } }));
    await flush();
    expect(calls()).toBe(before + 2);
  });
});
