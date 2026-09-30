// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { GroupStatus } from "../../lib/types";

const api = vi.hoisted(() => ({ setCivitaiKey: vi.fn<(k: string) => Promise<void>>() }));
vi.mock("../../lib/api", async (orig) => ({ ...(await orig<typeof import("../../lib/api")>()), ...api }));

import { ApiKeyDialog, GroupProgress } from "./controls";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ApiKeyDialog", () => {
  it("saves once when Enter is pressed again while saving", async () => {
    let finish!: () => void;
    api.setCivitaiKey.mockImplementation(() => new Promise<void>((r) => (finish = r)));
    const onSaved = vi.fn();
    render(<ApiKeyDialog open onClose={() => undefined} onSaved={onSaved} />);
    const input = screen.getByLabelText("API key");
    fireEvent.change(input, { target: { value: "0123456789abcdef0123456789" } });
    const form = input.closest("form")!;
    fireEvent.submit(form);
    fireEvent.submit(form);
    fireEvent.click(screen.getByRole("button", { name: /Save key/ }));
    expect(api.setCivitaiKey).toHaveBeenCalledTimes(1);
    await act(async () => finish());
    expect(onSaved).toHaveBeenCalledTimes(1);
  });

  it("gives the show/hide button the shared focus ring", () => {
    render(<ApiKeyDialog open onClose={() => undefined} onSaved={() => undefined} />);
    expect(screen.getByRole("button", { name: "Show key" }).className).toContain("focus-visible:ring-2");
  });
});

describe("GroupProgress", () => {
  it("gives Cancel the shared focus ring", () => {
    const g: GroupStatus = {
      groupId: "g",
      label: "Model",
      kind: "model",
      state: "downloading",
      currentFile: null,
      fileIndex: 0,
      fileCount: 1,
      downloadedBytes: 1,
      totalBytes: 10,
      error: null,
    };
    render(<GroupProgress group={g} onCancel={() => undefined} />);
    expect(screen.getByRole("button", { name: "Cancel" }).className).toContain("focus-visible:ring-2");
  });
});
