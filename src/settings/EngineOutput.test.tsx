// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

const engineOutput = vi.fn();
vi.mock("../lib/api", () => ({
  engineOutput: () => engineOutput(),
  asCoreError: (e: unknown) => ({ code: "internal", message: String(e) }),
}));

import { EngineOutput } from "./EngineOutput";

afterEach(() => {
  cleanup();
  engineOutput.mockReset();
});

describe("EngineOutput", () => {
  it("fetches the engine output only when asked, and hides it again", async () => {
    engineOutput.mockResolvedValue("[INFO ] backend_fit.cpp:326  -     CUDA0        NVIDIA GeForce RTX 5070 Ti");
    render(<EngineOutput />);
    expect(engineOutput).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText("Show engine output"));
    expect(await screen.findByText(/NVIDIA GeForce RTX 5070 Ti/)).toBeTruthy();
    fireEvent.click(screen.getByText("Hide engine output"));
    expect(screen.queryByText(/RTX 5070 Ti/)).toBeNull();
  });

  it("says so when the engine hasn't run yet", async () => {
    engineOutput.mockResolvedValue("");
    render(<EngineOutput />);
    fireEvent.click(screen.getByText("Show engine output"));
    expect(await screen.findByText(/hasn't run since Pinhole started/)).toBeTruthy();
  });

  it("drops an answer that arrives after Hide", async () => {
    let resolve: (s: string) => void = () => undefined;
    engineOutput.mockReturnValue(new Promise<string>((r) => (resolve = r)));
    render(<EngineOutput />);
    fireEvent.click(screen.getByText("Show engine output"));
    // Still loading: the button says Show; a second click starts a newer request.
    engineOutput.mockResolvedValue("newer");
    fireEvent.click(screen.getByText("Show engine output"));
    expect(await screen.findByText("newer")).toBeTruthy();
    fireEvent.click(screen.getByText("Hide engine output"));
    resolve("older");
    await Promise.resolve();
    expect(screen.queryByText("older")).toBeNull();
    expect(screen.getByText("Show engine output")).toBeTruthy();
  });
});
