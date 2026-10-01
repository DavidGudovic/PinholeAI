// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

vi.mock("../lib/api", async (orig) => ({ ...(await orig<typeof import("../lib/api")>()), openExternalLink: vi.fn(async () => undefined) }));

const { installMocks } = await import("../lib/mock");
const { getSettings, openExternalLink } = await import("../lib/api");
const { SettingsSheet } = await import("./SettingsSheet");

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

  it("shows the usage guidelines and the licence in the app, and opens GitHub to report a problem", async () => {
    render(<SettingsSheet open onClose={() => undefined} />);
    fireEvent.click(await screen.findByRole("button", { name: "Licence" }));
    expect(await screen.findByText(/Pinhole Licence 1\.0/)).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: "Close" }).at(-1)!);
    fireEvent.click(screen.getByRole("button", { name: "Usage guidelines" }));
    expect(await screen.findByText("Not allowed")).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: "Close" }).at(-1)!);
    fireEvent.click(screen.getByRole("button", { name: "Report privately" }));
    expect(openExternalLink).toHaveBeenLastCalledWith("https://github.com/DavidGudovic/PinholeAI/security/advisories/new");
    fireEvent.click(screen.getByRole("button", { name: "Open a public issue" }));
    expect(openExternalLink).toHaveBeenLastCalledWith("https://github.com/DavidGudovic/PinholeAI/issues/new/choose");
  });
});
