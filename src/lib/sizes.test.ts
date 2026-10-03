import { describe, expect, it } from "vitest";
import { namedSizes, referenceSize, sizeForRatio } from "./sizes";

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

describe("same as reference", () => {
  const on = { refShape: true, refImageId: "r" };
  it("uses the reference picture's shape at the model's area", () => {
    expect(referenceSize(on, { width: 3000, height: 2000 }, null)).toEqual([1280, 832]);
    expect(referenceSize(on, { width: 500, height: 500 }, { shapes: { square: [512, 512] } })).toEqual([512, 512]);
  });

  it("keeps very long pictures within 3:1, like the sizes Pinhole makes", () => {
    expect(referenceSize(on, { width: 4000, height: 1000 }, null)).toEqual([1728, 576]);
    expect(referenceSize(on, { width: 100, height: 2000 }, null)).toEqual([576, 1728]);
    for (const r of [2.9, 3, 3.2, 1 / 3.2, 1 / 2.9]) {
      const [w, h] = sizeForRatio(r, null);
      expect(Math.max(w, h) / Math.min(w, h)).toBeLessThanOrEqual(3);
    }
  });

  it("is off without the choice, the picture or a usable size", () => {
    expect(referenceSize({ ...on, refShape: false }, { width: 3000, height: 2000 }, null)).toBeNull();
    expect(referenceSize({ ...on, refImageId: null }, { width: 3000, height: 2000 }, null)).toBeNull();
    expect(referenceSize(on, null, null)).toBeNull();
    expect(referenceSize(on, { width: 0, height: 10 }, null)).toBeNull();
  });
});
