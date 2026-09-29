// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { installMocks } from "../lib/mock";
import { getSettings } from "../lib/api";
import { SettingsSheet } from "./SettingsSheet";

beforeAll(async () => {
  await installMocks();
});
afterEach(cleanup);

describe("SettingsSheet", () => {
  it("shows the text-encoder choice (auto / on / off) and saves it", async () => {
    render(<SettingsSheet open onClose={() => undefined} />);
    // Shown once a graphics card is known (the mock detects an RTX 5070 Ti after ~1.3 s).
    const label = "Read the prompt on the processor";
    await screen.findByRole("radiogroup", { name: label }, { timeout: 5000 });
    const radio = (name: string) => within(screen.getByRole("radiogroup", { name: label })).getByRole("radio", { name });
    expect(radio("Automatic").getAttribute("aria-checked")).toBe("true");
    expect(radio("Off").getAttribute("aria-checked")).toBe("false");

    fireEvent.click(radio("On"));
    await waitFor(async () => expect((await getSettings()).textEncoderOnCpu).toBe("on"));
    await waitFor(() => expect(radio("On").getAttribute("aria-checked")).toBe("true"));

    fireEvent.click(radio("Off"));
    await waitFor(async () => expect((await getSettings()).textEncoderOnCpu).toBe("off"));
  }, 10_000);

  it("names every choice group for screen readers", async () => {
    render(<SettingsSheet open onClose={() => undefined} />);
    expect(await screen.findByRole("radiogroup", { name: "Theme" })).toBeTruthy();
    expect(screen.getByRole("radiogroup", { name: "Information inside saved pictures" })).toBeTruthy();
    expect(screen.getByRole("radiogroup", { name: "Safe mode" })).toBeTruthy();
  });

  it("doesn't show an old 'Saved' tick after closing right after a change", async () => {
    const { rerender } = render(<SettingsSheet open onClose={() => undefined} />);
    fireEvent.click(await screen.findByRole("switch", { name: "Show paid models" }));
    await screen.findByText("Saved");
    // Closed well within the 1.8 s the tick stays up.
    rerender(<SettingsSheet open={false} onClose={() => undefined} />);
    rerender(<SettingsSheet open onClose={() => undefined} />);
    await screen.findByRole("radiogroup", { name: "Theme" });
    expect(screen.queryByText("Saved")).toBeNull();
    // Put the setting back for the other tests.
    fireEvent.click(screen.getByRole("switch", { name: "Show paid models" }));
    await screen.findByText("Saved");
  });
});
