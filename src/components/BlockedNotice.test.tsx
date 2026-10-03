// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";

const BLOCKED = { code: "blocked", message: "Someone may look under 18. The age check can be wrong about young-looking adults.", details: null };
const core = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => core);

import { browseCatalog, generate, previewFinalPrompt } from "../lib/api";
import { dismissBlocked } from "../lib/blocked";
import type { GenerateRequest } from "../lib/types";
import { BlockedNotice } from "./BlockedNotice";

afterEach(() => {
  act(() => dismissBlocked());
  cleanup();
});

beforeEach(() => {
  core.invoke.mockImplementation(async () => Promise.reject(BLOCKED));
});

describe("BlockedNotice", () => {
  it("opens the usage guidelines with the block message when the check stops something", async () => {
    render(<BlockedNotice />);
    expect(screen.queryByText("Usage guidelines")).toBeNull();
    await act(async () => {
      await expect(generate({} as GenerateRequest)).rejects.toMatchObject({ code: "blocked" });
    });
    expect(screen.getByRole("status").textContent).toBe(`Why it was blocked: ${BLOCKED.message}`);
    expect(screen.getByText("Not allowed")).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: "Close" })[0]);
    expect(screen.queryByText("Usage guidelines")).toBeNull();
  });

  it("stays closed for calls made while typing (prompt preview, Browse search)", async () => {
    render(<BlockedNotice />);
    await act(async () => {
      await expect(previewFinalPrompt({} as GenerateRequest)).rejects.toMatchObject({ code: "blocked" });
      await expect(browseCatalog({} as never)).rejects.toMatchObject({ code: "blocked" });
    });
    expect(screen.queryByText("Usage guidelines")).toBeNull();
  });

  it("stays closed for other errors", async () => {
    core.invoke.mockImplementation(async () => Promise.reject({ code: "io", message: "Disk full.", details: null }));
    render(<BlockedNotice />);
    await act(async () => {
      await expect(generate({} as GenerateRequest)).rejects.toMatchObject({ code: "io" });
    });
    expect(screen.queryByText("Usage guidelines")).toBeNull();
  });
});
