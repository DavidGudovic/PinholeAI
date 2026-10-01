// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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

  it("offers Delete only for helpers Pinhole downloaded", async () => {
    list = [helper({ installed: true, removable: true, downloadBytes: 0 }), helper({ id: "qwen25_vl_7b", title: "Qwen2.5-VL 7B", installed: true, removable: false, downloadBytes: 0 })];
    show();
    await screen.findByText("Qwen2.5-VL 7B");
    expect(screen.getAllByRole("button", { name: /Delete/ })).toHaveLength(1);
    expect(screen.getByText("Came with another model")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Delete/ }));
    fireEvent.click(await screen.findByRole("button", { name: /Delete · frees/ }));
    await waitFor(() => expect(api.deleteHelper).toHaveBeenCalledWith("describe"));
  });
});

describe("Describe model picker", () => {
  it("offers Automatic and installed helpers only, and saves the choice", async () => {
    list = [
      helper({ installed: true, downloadBytes: 0 }),
      helper({ id: "qwen25_vl_7b", title: "Qwen2.5-VL 7B" }),
      helper({ id: "qwen25_vl_7b_safe_off", title: "Qwen2.5-VL 7B abliterated", installed: true, downloadBytes: 0, needsSafeOff: true }),
    ];
    render(<AppProvider store={createStore()}><HelperPicker purpose="describe" /></AppProvider>);
    fireEvent.click(await screen.findByRole("button", { name: "Model: Automatic" }));
    const menu = within(screen.getByRole("listbox", { name: "Model" }));
    expect(menu.getAllByRole("menuitem")).toHaveLength(3);
    expect(menu.getByText("Automatic")).toBeTruthy();
    expect(menu.getByText("Get more models…")).toBeTruthy();
    expect(menu.queryByText("Qwen2.5-VL 7B")).toBeNull();
    // Safe mode is On (the default): the Safe-mode-Off helper is never offered.
    expect(menu.queryByText("Qwen2.5-VL 7B abliterated")).toBeNull();
    fireEvent.click(screen.getByText("Qwen2.5-VL 3B"));
    await waitFor(async () => expect((await api.getSettings()).describeModel).toBe("describe"));
  });
});
