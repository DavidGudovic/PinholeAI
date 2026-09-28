import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { UpdateCheck } from "../lib/types";

const api = vi.hoisted(() => ({
  checkForUpdates: vi.fn<() => Promise<UpdateCheck>>(),
  installUpdate: vi.fn<(v: string) => Promise<void>>(),
  openReleasePage: vi.fn<(v: string | null) => Promise<void>>(),
}));
vi.mock("../lib/api", async (orig) => ({ ...(await orig<typeof import("../lib/api")>()), ...api }));
vi.mock("../tabs/models/lib/downloads", () => ({ useTaggedGroup: () => null, cancelGroup: vi.fn() }));

import { UpdateSection } from "./UpdateSection";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const update = (installMode: "installer" | "manual"): UpdateCheck => ({
  currentVersion: "0.1.0",
  update: { version: "0.2.0", publishedAt: null, installMode, sizeBytes: installMode === "manual" ? null : 14 * 1024 * 1024 },
});

describe("UpdateSection", () => {
  it("never checks by itself and can't check while offline", () => {
    render(<UpdateSection offline />);
    expect(api.checkForUpdates).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: /check for updates/i })).toHaveProperty("disabled", true);
  });

  it("says when Pinhole is up to date", async () => {
    api.checkForUpdates.mockResolvedValue({ currentVersion: "0.1.0", update: null });
    render(<UpdateSection offline={false} />);
    fireEvent.click(screen.getByRole("button", { name: /check for updates/i }));
    expect(await screen.findByText(/newest version \(0\.1\.0\)/)).toBeTruthy();
  });

  it("offers Update and restart, and installs the checked version", async () => {
    api.checkForUpdates.mockResolvedValue(update("installer"));
    api.installUpdate.mockReturnValue(new Promise(() => undefined));
    render(<UpdateSection offline={false} />);
    fireEvent.click(screen.getByRole("button", { name: /check for updates/i }));
    fireEvent.click(await screen.findByRole("button", { name: /update and restart/i }));
    expect(api.installUpdate).toHaveBeenCalledWith("0.2.0");
  });

  it("sends copies that can't update themselves to the download page", async () => {
    api.checkForUpdates.mockResolvedValue(update("manual"));
    api.openReleasePage.mockResolvedValue();
    render(<UpdateSection offline={false} />);
    fireEvent.click(screen.getByRole("button", { name: /check for updates/i }));
    fireEvent.click(await screen.findByRole("button", { name: /open download page/i }));
    expect(api.openReleasePage).toHaveBeenCalledWith("0.2.0");
    expect(screen.queryByRole("button", { name: /update and restart/i })).toBeNull();
  });

  it("shows a plain error when the check fails", async () => {
    api.checkForUpdates.mockRejectedValue({ code: "network", message: "Network problem — check your connection and try again.", details: null });
    render(<UpdateSection offline={false} />);
    fireEvent.click(screen.getByRole("button", { name: /check for updates/i }));
    expect(await screen.findByText(/Network problem/)).toBeTruthy();
  });
});
