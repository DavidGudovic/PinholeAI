// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { HelperModel } from "../../lib/types";

const helper = (over: Partial<HelperModel>): HelperModel => ({
  id: "describe",
  title: "Qwen2.5-VL 3B",
  note: "Small and quick.",
  sizeBytes: 2_775_000_000,
  downloadBytes: 2_775_000_000,
  installed: false,
  removable: false,
  fit: "fits",
  needsSafeOff: false,
  ...over,
});
let list: HelperModel[] = [];
vi.mock("../../lib/api", async (orig) => {
  const real = await orig<typeof import("../../lib/api")>();
  return {
    ...real,
    listHelperModels: vi.fn(async () => list),
    installCaptioner: vi.fn(async () => ({ groupId: "g" })),
    deleteHelper: vi.fn(async () => undefined),
  };
});

const api = await import("../../lib/api");
const { installMocks } = await import("../../lib/mock");
const { AppProvider } = await import("../../lib/state/AppProvider");
const { createStore } = await import("../../lib/state/store");
const { HelpersView } = await import("./HelpersView");
const { HelperPicker } = await import("../../components/HelperPicker");

beforeAll(async () => {
  await installMocks();
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const show = () => render(<AppProvider store={createStore()}><HelpersView /></AppProvider>);

describe("Helpers view", () => {
  it("lists helper models with size and fit, and gets one", async () => {
    list = [helper({}), helper({ id: "qwen25_vl_7b", title: "Qwen2.5-VL 7B", sizeBytes: 8_952_000_000, downloadBytes: 8_952_000_000, fit: "tight" })];
    show();
    await screen.findByText("Qwen2.5-VL 3B");
    expect(screen.getByText("Fits")).toBeTruthy();
    expect(screen.getByText("Tight")).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: /Get/ })[1]);
    await waitFor(() => expect(api.installCaptioner).toHaveBeenCalledWith("qwen25_vl_7b"));
  });

  it("offers Remove only for helpers Pinhole downloaded", async () => {
    list = [helper({ installed: true, removable: true, downloadBytes: 0 }), helper({ id: "qwen25_vl_7b", title: "Qwen2.5-VL 7B", installed: true, removable: false, downloadBytes: 0 })];
    show();
    await screen.findByText("Qwen2.5-VL 7B");
    expect(screen.getAllByRole("button", { name: /Remove/ })).toHaveLength(1);
    expect(screen.getByText("Came with another model")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Remove/ }));
    fireEvent.click(await screen.findByRole("button", { name: /Remove · frees/ }));
    await waitFor(() => expect(api.deleteHelper).toHaveBeenCalledWith("describe"));
  });
});

describe("Helper picker", () => {
  it("offers Automatic and installed helpers only, and saves the choice", async () => {
    list = [helper({ installed: true, downloadBytes: 0 }), helper({ id: "qwen25_vl_7b", title: "Qwen2.5-VL 7B" })];
    render(<AppProvider store={createStore()}><HelperPicker purpose="improve" /></AppProvider>);
    const select = (await screen.findByLabelText("Model for Improve")) as HTMLSelectElement;
    const labels = Array.from(select.options).map((o) => o.text);
    expect(labels).toEqual(["Model: Automatic", "Qwen2.5-VL 3B", "Get more models…"]);
    fireEvent.change(select, { target: { value: "describe" } });
    await waitFor(async () => expect((await api.getSettings()).improveModel).toBe("describe"));
  });
});
