// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { SafetyCheckStatus } from "../lib/types";

const api = vi.hoisted(() => ({ installSafetyCheck: vi.fn<() => Promise<SafetyCheckStatus>>() }));
vi.mock("../lib/api", async (orig) => ({ ...(await orig<typeof import("../lib/api")>()), ...api }));

import { installMocks } from "../lib/mock";
import { AppProvider } from "../lib/state/AppProvider";
import { ErrorWithFix } from "./ErrorWithFix";

beforeAll(async () => {
  await installMocks();
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const MISSING = {
  code: "check_missing",
  message: "Pinhole's safety check isn't set up yet. Click “Set up safety check” to download it (about 1.1 GB), then try again.",
  details: null,
};

describe("ErrorWithFix", () => {
  it("offers to set up the safety check, then to try again", async () => {
    api.installSafetyCheck.mockResolvedValue({ ready: true, downloading: false, downloadBytes: 0 });
    const retry = vi.fn();
    render(
      <AppProvider>
        <ErrorWithFix error={MISSING} onRetry={retry} />
      </AppProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: /Set up safety check/ }));
    fireEvent.click(await screen.findByRole("button", { name: /Try again/ }));
    expect(api.installSafetyCheck).toHaveBeenCalledTimes(1);
    expect(retry).toHaveBeenCalledTimes(1);
  });

  it("shows a failed download and lets you try the setup again", async () => {
    api.installSafetyCheck.mockRejectedValue({ code: "offline", message: "Offline mode is on. Turn it off in Settings to browse or download.", details: null });
    render(
      <AppProvider>
        <ErrorWithFix error={MISSING} />
      </AppProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: /Set up safety check/ }));
    expect(await screen.findByText(/Offline mode is on/)).toBeTruthy();
    expect(screen.getByRole("button", { name: /Set up safety check/ })).toBeTruthy();
  });
});
