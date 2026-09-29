// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { installMocks } from "../lib/mock";
import { getSettings, setSettings } from "../lib/api";
import { ModelsFolderSection } from "./ModelsFolderSection";

beforeAll(async () => {
  await installMocks();
});
afterEach(cleanup);

describe("ModelsFolderSection", () => {
  it("picks a folder, shows what moves, and moves the models", async () => {
    render(<ModelsFolderSection />);
    await screen.findByText(/Data\\models$/);
    fireEvent.click(screen.getByRole("button", { name: /Change/ }));
    await screen.findByText("Move your models to this folder?");
    expect(screen.getByText(/already has 1 model from another Pinhole/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Move and restart" }));
    await screen.findByText("Restarting Pinhole…", undefined, { timeout: 5000 });
    expect((await getSettings()).modelsFolder).toBe("D:\\Shared\\Pinhole Models");
  }, 10_000);

  it("a plain settings save can't change the Models folder", async () => {
    const s = await getSettings();
    const saved = await setSettings({ ...s, modelsFolder: "/somewhere/else" });
    expect(saved.modelsFolder).toBe(s.modelsFolder);
    await waitFor(async () => expect((await getSettings()).modelsFolder).toBe(s.modelsFolder));
  });
});
