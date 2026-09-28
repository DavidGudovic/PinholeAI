import { describe, expect, it } from "vitest";
import { fitWords } from "./format";

describe("fitWords", () => {
  it("talks about VRAM on a GPU", () => {
    expect(fitWords({ gb: 6, estimate: false }, "tooBig")).toMatchObject({ need: "Needs ~6 GB VRAM", short: "~6 GB VRAM", badge: "Too big", tone: "red" });
    expect(fitWords({ gb: 9.4, estimate: true, onCpu: false }, "fits")).toMatchObject({ need: "Needs ~9.4 GB VRAM (estimate)", badge: "Fits", tone: "green" });
    expect(fitWords({ gb: 16, estimate: false }, "tight").badge).toBe("Tight");
    expect(fitWords({ gb: 16, estimate: false }, null).badge).toBeNull();
  });

  it("talks about memory and the processor without a GPU", () => {
    const w = fitWords({ gb: 3, estimate: true, onCpu: true }, "tight");
    expect(w).toMatchObject({ need: "Needs ~3 GB memory", short: "~3 GB RAM", badge: "Slow", tone: "amber" });
    expect(w.title).toContain("processor");
    expect(w.need).not.toContain("VRAM");
    const big = fitWords({ gb: 22, estimate: true, onCpu: true }, "tooBig");
    expect(big.badge).toBe("Too big");
    expect(big.title).toContain("too big to run on the processor");
  });
});
