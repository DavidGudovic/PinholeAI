// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import type { InstalledLora, InstalledModel } from "../../lib/types";

vi.mock("../../lib/api", async (orig) => {
  const real = await orig<typeof import("../../lib/api")>();
  return { ...real, listModels: vi.fn(real.listModels) };
});

const api = await import("../../lib/api");
const { installMocks } = await import("../../lib/mock");
const { AppProvider } = await import("../../lib/state/AppProvider");
const { createStore } = await import("../../lib/state/store");
const { ModelsTab } = await import("./ModelsTab");

beforeAll(async () => {
  await installMocks();
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const model = (id: string) => ({ id, friendlyName: id, familyId: null, modes: ["txt2img"], isEditModel: false, fit: null, missingComponents: [] }) as unknown as InstalledModel;
const flush = () => act(() => new Promise((r) => setTimeout(r, 0)));

describe("Models tab", () => {
  it("counts installed models and add-ons from the app's model list", async () => {
    const store = createStore();
    render(
      <AppProvider store={store}>
        <ModelsTab />
      </AppProvider>,
    );
    await waitFor(() => expect(store.getState().models).not.toBeNull());
    await flush();
    const calls = vi.mocked(api.listModels).mock.calls.length;

    act(() => {
      store.dispatch({ type: "setModels", models: [model("a"), model("b")] });
      store.dispatch({ type: "setLoras", loras: [{ id: "l" }] as InstalledLora[] });
    });
    // The label follows the store without fetching the list again.
    expect(screen.getByRole("radio", { name: "Installed (3)" })).toBeTruthy();
    expect(api.listModels).toHaveBeenCalledTimes(calls);
  });
});
