// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { installMocks } from "../lib/mock";
import { getSettings, installCaptioner } from "../lib/api";
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
});
