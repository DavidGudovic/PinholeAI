// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { installMocks } from "../lib/mock";
import { getSettings } from "../lib/api";
import { SaveFolderSection } from "./SaveFolderSection";

beforeAll(async () => {
  await installMocks();
});
afterEach(cleanup);

describe("Settings → Saved pictures folder", () => {
  it("shows Pictures/Pinhole, changes to a picked folder and back", async () => {
    render(<SaveFolderSection />);
    expect(await screen.findByText("C:\\Users\\Alex\\Pictures\\Pinhole")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Use default/ })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /Change…/ }));
    expect(await screen.findByText("C:\\Users\\Alex\\Pictures\\Wallpapers")).toBeTruthy();
    // Stored with the other settings (a path, nothing else).
    expect((await getSettings()).saveFolder).toBe("C:\\Users\\Alex\\Pictures\\Wallpapers");

    fireEvent.click(await screen.findByRole("button", { name: /Use default/ }));
    expect(await screen.findByText("C:\\Users\\Alex\\Pictures\\Pinhole")).toBeTruthy();
    expect((await getSettings()).saveFolder).toBeNull();
  });

  it("says why a picked folder can't be used and keeps the old one", async () => {
    const api = await import("../lib/api");
    const spy = vi.spyOn(api, "setSaveFolder").mockRejectedValueOnce({ code: "invalid", message: "Pinhole can't save into that folder. Pick one you can write to.", details: null });
    render(<SaveFolderSection />);
    expect(await screen.findByText("C:\\Users\\Alex\\Pictures\\Pinhole")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Change…/ }));
    expect(await screen.findByText(/can't save into that folder/)).toBeTruthy();
    expect(screen.getByText("C:\\Users\\Alex\\Pictures\\Pinhole")).toBeTruthy();
    spy.mockRestore();
  });
});
