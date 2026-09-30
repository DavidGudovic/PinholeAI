// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";

const BLOCKED = { code: "blocked", message: "Pinhole doesn't make sexual images or text involving anyone under 18.", details: null };
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => Promise.reject(BLOCKED)) }));

import { generate, previewFinalPrompt } from "../lib/api";
import { dismissBlocked } from "../lib/blocked";
import type { GenerateRequest } from "../lib/types";
import { BlockedNotice } from "./BlockedNotice";

afterEach(() => {
  act(() => dismissBlocked());
  cleanup();
});

describe("BlockedNotice", () => {
  it("opens the usage guidelines with the block message when the check stops something", async () => {
    render(<BlockedNotice />);
    expect(screen.queryByText("Usage guidelines")).toBeNull();
    await act(async () => {
      await expect(generate({} as GenerateRequest)).rejects.toMatchObject({ code: "blocked" });
    });
    expect(screen.getByRole("status").textContent).toBe(BLOCKED.message);
    expect(screen.getByText("Not allowed")).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: "Close" })[0]);
    expect(screen.queryByText("Usage guidelines")).toBeNull();
  });

  it("stays closed for the prompt preview while typing", async () => {
    render(<BlockedNotice />);
    await act(async () => {
      await expect(previewFinalPrompt({} as GenerateRequest)).rejects.toMatchObject({ code: "blocked" });
    });
    expect(screen.queryByText("Usage guidelines")).toBeNull();
  });
});
