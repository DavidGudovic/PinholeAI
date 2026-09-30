import { describe, expect, it } from "vitest";
import { namedSizes, sizeForRatio } from "./sizes";

describe("named sizes", () => {
  it("keeps the model's picture area and snaps to 64", () => {
    expect(sizeForRatio(9 / 16, null)).toEqual([768, 1344]);
    expect(sizeForRatio(4 / 5, null)).toEqual([896, 1152]);
    expect(sizeForRatio(16 / 9, null)).toEqual([1344, 768]);
  });

  it("follows the model's own Square size", () => {
    expect(sizeForRatio(16 / 9, { shapes: { square: [512, 512] } })).toEqual([704, 384]);
  });

  it("stays inside Fine-tune's limits for extreme screens", () => {
    const [w, h] = sizeForRatio(32 / 9, null);
    expect(w).toBeLessThanOrEqual(4096);
    expect(h).toBeGreaterThanOrEqual(256);
  });

  it("offers My screen first only when the screen size is known", () => {
    expect(namedSizes(null).map((s) => s.id)).toEqual(["phone", "instagram", "thumbnail"]);
    const withScreen = namedSizes({ width: 2560, height: 1440 });
    expect(withScreen.map((s) => s.id)).toEqual(["screen", "phone", "instagram", "thumbnail"]);
    expect(withScreen[0].ratio).toBeCloseTo(16 / 9);
  });
});
