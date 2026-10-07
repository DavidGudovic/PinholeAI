// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { EngineStatus, GroupStatus } from "../lib/types";

const api = vi.hoisted(() => ({ installEngine: vi.fn<() => Promise<EngineStatus>>() }));
vi.mock("../lib/api", async (orig) => ({ ...(await orig<typeof import("../lib/api")>()), ...api }));

import { installMocks, mockEmit } from "../lib/mock";
import { AppProvider } from "../lib/state/AppProvider";
import { DownloadsPanel } from "../tabs/models/DownloadsPanel";
import { TopBar } from "./TopBar";

beforeAll(async () => {
  await installMocks();
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const finished = (groupId: string): GroupStatus => ({
  groupId,
  label: `Finished ${groupId}`,
  kind: "model",
  state: "done",
  currentFile: null,
  fileIndex: 0,
  fileCount: 1,
  downloadedBytes: 100,
  totalBytes: 100,
  error: null,
});

describe("TopBar", () => {
  it("a click on a section puts the cursor in its text field; arrow keys move between sections", async () => {
    const { createStore } = await import("../lib/state/store");
    const store = createStore();
    render(
      <AppProvider store={store}>
        <TopBar onOpenSettings={() => undefined} />
        <div id="tab-create" hidden={store.getState().tab !== "create"}>
          <textarea id="prompt" aria-label="Prompt" />
        </div>
      </AppProvider>,
    );
    store.dispatch({ type: "setTab", tab: "models" });
    fireEvent.click(screen.getByRole("tab", { name: /Create/ }), { detail: 1 });
    expect(store.getState().tab).toBe("create");
    await waitFor(() => expect(document.activeElement).toBe(screen.getByLabelText("Prompt")));

    const create = screen.getByRole("tab", { name: /Create/ });
    create.focus();
    fireEvent.keyDown(create, { key: "ArrowRight" });
    expect(store.getState().tab).toBe("edit");
    expect(document.activeElement).toBe(screen.getByRole("tab", { name: /Edit/ }));
    fireEvent.keyDown(document.activeElement!, { key: "ArrowLeft" });
    fireEvent.keyDown(document.activeElement!, { key: "ArrowLeft" });
    expect(store.getState().tab).toBe("models");
  });

  it("keeps a failed engine download on screen with its Details", async () => {
    api.installEngine.mockRejectedValue({ code: "download", message: "The engine download failed. Check your connection and try again.", details: "HTTP 503 from github.com" });
    render(
      <AppProvider>
        <TopBar onOpenSettings={() => undefined} />
      </AppProvider>,
    );
    fireEvent.click(await screen.findByRole("button", { name: /Get the engine/ }, { timeout: 5000 }));
    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText(/engine download failed/)).toBeTruthy();
    // The engine output is behind a Details toggle, like Settings → Engine.
    fireEvent.click(within(alert).getByRole("button", { name: /Details/ }));
    expect(await within(alert).findByText(/HTTP 503/)).toBeTruthy();
  }, 10_000);

  it("clears finished downloads from both lists with either button", async () => {
    render(
      <AppProvider>
        <TopBar onOpenSettings={() => undefined} />
        <DownloadsPanel />
      </AppProvider>,
    );
    mockEmit("download-progress", finished("f1"));
    const panel = await screen.findByRole("region", { name: "Downloads" });
    await within(panel).findByText("Finished f1");
    // Top bar's button also clears the Models tab list.
    fireEvent.click(await screen.findByRole("button", { name: "Downloads" }));
    await waitFor(() => expect(screen.getAllByRole("button", { name: "Clear finished" })).toHaveLength(2));
    fireEvent.click(screen.getAllByRole("button", { name: "Clear finished" }).find((b) => !panel.contains(b))!);
    await waitFor(() => expect(screen.queryByRole("region", { name: "Downloads" })).toBeNull());
    await waitFor(() => expect(screen.queryByRole("button", { name: "Downloads" })).toBeNull());

    // And the Models tab's button also clears the top bar list.
    mockEmit("download-progress", finished("f2"));
    const panel2 = await screen.findByRole("region", { name: "Downloads" });
    await screen.findByRole("button", { name: "Downloads" });
    fireEvent.click(within(panel2).getByRole("button", { name: "Clear finished" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Downloads" })).toBeNull());
  }, 10_000);
});
