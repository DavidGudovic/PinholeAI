// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { Settings } from "../lib/types";

const api = vi.hoisted(() => ({
  getSettings: vi.fn(async () => ({ theme: "system", noticeAccepted: 0 }) as unknown as Settings),
  setSettings: vi.fn(async (s: Settings) => s),
  quitApp: vi.fn(async () => undefined),
}));
vi.mock("../lib/api", async (orig) => ({ ...(await orig<typeof import("../lib/api")>()), ...api }));

import { NOTICE_VERSION, UseNotice } from "./UseNotice";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("Before you start", () => {
  it("says what the check does, that it can't be turned off, and stores only the notice version", async () => {
    const onAgreed = vi.fn();
    render(<UseNotice onAgreed={onAgreed} />);
    expect(screen.getByText(/can't be turned off/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "usage guidelines" }));
    expect(screen.getByText(/Sexual content involving anyone under 18/)).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: "Close" })[0]);
    expect(screen.queryByText(/Sexual content involving/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Agree and continue/ }));
    await waitFor(() => expect(onAgreed).toHaveBeenCalled());
    expect(api.setSettings).toHaveBeenCalledWith(expect.objectContaining({ theme: "system", noticeAccepted: NOTICE_VERSION }));
  });

  it("continues for this session when settings can't be saved", async () => {
    api.getSettings.mockRejectedValueOnce({ code: "io", message: "x", details: null });
    const onAgreed = vi.fn();
    render(<UseNotice onAgreed={onAgreed} />);
    fireEvent.click(screen.getByRole("button", { name: /Agree and continue/ }));
    await waitFor(() => expect(onAgreed).toHaveBeenCalled());
  });

  it("Quit closes the app without agreeing", () => {
    const onAgreed = vi.fn();
    render(<UseNotice onAgreed={onAgreed} />);
    fireEvent.click(screen.getByRole("button", { name: "Quit" }));
    expect(api.quitApp).toHaveBeenCalled();
    expect(api.setSettings).not.toHaveBeenCalled();
    expect(onAgreed).not.toHaveBeenCalled();
  });
});
