// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { installMocks } from "../lib/mock";
import { getSettings, installCaptioner } from "../lib/api";
import { askLicence } from "../lib/licence";
import { LicencePrompt } from "./LicencePrompt";

beforeAll(async () => {
  await installMocks();
});
afterEach(cleanup);

describe("LicencePrompt", () => {
  it("asks once per licence before a licensed download", async () => {
    render(<LicencePrompt />);

    // Cancel: nothing is downloaded and nothing is accepted.
    const declined = installCaptioner();
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));
    await expect(declined).rejects.toMatchObject({ code: "cancelled" });
    expect((await getSettings()).acceptedLicenses ?? []).not.toContain("qwen-research");

    // I accept: the acceptance is saved and the download starts.
    const accepted = installCaptioner();
    expect(await screen.findByText(/Qwen Research License/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "I accept" }));
    expect((await accepted).groupId).toBeTruthy();
    expect((await getSettings()).acceptedLicenses).toContain("qwen-research");

    // Accepted licences aren't asked again.
    expect((await installCaptioner()).groupId).toBeTruthy();
    expect(screen.queryByRole("button", { name: "I accept" })).toBeNull();
  });

  it("shares one prompt for the same licence and queues other licences", async () => {
    render(<LicencePrompt />);
    // "Get all" can start two installs with the same licence at once.
    const a = askLicence("lic-a", "This model comes with its own licence: A.");
    const b = askLicence("lic-a", "This model comes with its own licence: A.");
    const c = askLicence("lic-c", "This model comes with its own licence: C.");
    expect(await screen.findByText(/licence: A\./)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "I accept" }));
    expect(await a).toBe(true);
    expect(await b).toBe(true);
    // The other licence is asked next, not declined.
    expect(await screen.findByText(/licence: C\./)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await c).toBe(false);
    expect(screen.queryByRole("button", { name: "I accept" })).toBeNull();
  });
});
