import { describe, expect, it, vi } from "vitest";
import { clearGenerationHandoff, onGenerationHandoff, sendGenerationToCreate } from "./handoff";

describe("generation handoff", () => {
  it("delivers pending text when Create mounts", () => {
    sendGenerationToCreate("steps: 8");
    const cb = vi.fn();
    const off = onGenerationHandoff(cb);
    expect(cb).toHaveBeenCalledWith("steps: 8");
    off();
  });

  it("forgets pending text on Reset", () => {
    sendGenerationToCreate("steps: 8");
    clearGenerationHandoff();
    const cb = vi.fn();
    const off = onGenerationHandoff(cb);
    expect(cb).not.toHaveBeenCalled();
    off();
  });
});
