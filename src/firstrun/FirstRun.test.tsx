// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { EngineStatus } from "../lib/types";

const api = vi.hoisted(() => ({
  engineStatus: vi.fn<() => Promise<EngineStatus>>(),
  installEngine: vi.fn<() => Promise<EngineStatus>>(),
  onEngine: vi.fn(() => Promise.resolve(() => undefined)),
  onDownload: vi.fn(() => Promise.resolve(() => undefined)),
  listDownloads: vi.fn(() => Promise.resolve([])),
}));
vi.mock("../lib/api", async (orig) => ({ ...(await orig<typeof import("../lib/api")>()), ...api }));

import { useEngine } from "../tabs/models/lib/hooks";
import { EngineStep } from "./FirstRun";

function Step() {
  const engine = useEngine();
  return <EngineStep engine={engine} hw={null} />;
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("First run engine step", () => {
  it("offers Try again when the engine check fails, and checks again", async () => {
    api.engineStatus.mockRejectedValueOnce({ code: "internal", message: "Pinhole couldn't check the engine. Try again.", details: null });
    api.engineStatus.mockResolvedValueOnce({ installed: false, backend: "cpu", version: "x", installing: false } as EngineStatus);
    render(<Step />);
    fireEvent.click(await screen.findByRole("button", { name: /Try again/ }));
    expect(api.engineStatus).toHaveBeenCalledTimes(2);
    // Nothing is downloaded until the check works.
    expect(api.installEngine).not.toHaveBeenCalled();
    expect(await screen.findByRole("button", { name: /Download engine/ })).toBeTruthy();
  });
});
