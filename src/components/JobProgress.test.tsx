import { describe, expect, it } from "vitest";
import type { GenerationProgress } from "../lib/types";
import type { Job } from "../lib/state/model";
import { jobStatusText } from "./JobProgress";

const job = (p: Partial<GenerationProgress> | null, kind: Job["kind"] = "create"): Job => ({
  kind,
  startedAt: 0,
  progress: p ? { phase: "generating", modelLabel: null, queuePosition: null, step: null, totalSteps: null, elapsedMs: 0, ...p } : null,
});

describe("jobStatusText", () => {
  it("shows model loading with a percentage when tensor counts are known", () => {
    expect(jobStatusText(job({ phase: "loadingModel", modelLabel: "Juggernaut XL", step: 565, totalSteps: 1130 }), 3)).toBe("Loading Juggernaut XL… 50%");
    expect(jobStatusText(job({ phase: "loadingModel", modelLabel: "Juggernaut XL" }), 3)).toBe("Loading Juggernaut XL… (~10–30 s)");
  });
  it("shows steps, queue position or elapsed time", () => {
    expect(jobStatusText(job({ phase: "generating", step: 7, totalSteps: 30 }), 2)).toBe("Creating · step 7 of 30");
    expect(jobStatusText(job({ phase: "generating" }, "edit"), 12)).toBe("Editing… 12 s");
    expect(jobStatusText(job({ phase: "queued", queuePosition: 2 }), 0)).toBe("Waiting in line (#2)…");
    expect(jobStatusText(job(null, "upscale"), 0)).toBe("Upscaling…");
  });
});
