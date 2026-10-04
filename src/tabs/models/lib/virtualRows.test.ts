import { describe, expect, it } from "vitest";
import { columnsFor, rowRange, rowTops } from "./virtualRows";

describe("columnsFor", () => {
  it("matches repeat(auto-fill, minmax(240px, 1fr)) with a 16px gap", () => {
    expect(columnsFor(239, 240, 16)).toBe(1);
    expect(columnsFor(240, 240, 16)).toBe(1);
    expect(columnsFor(495, 240, 16)).toBe(1);
    expect(columnsFor(496, 240, 16)).toBe(2);
    expect(columnsFor(1552, 240, 16)).toBe(6);
    expect(columnsFor(0, 240, 16)).toBe(1);
  });
});

describe("rowTops", () => {
  it("uses measured heights and the estimate for the rest", () => {
    expect(rowTops(3, [100, undefined, 50], 200, 10)).toEqual([0, 110, 320, 380]);
    expect(rowTops(0, [], 200, 10)).toEqual([0]);
  });
});

describe("rowRange", () => {
  const tops = rowTops(10, [], 100, 0); // rows of 100px: 0, 100, … 1000

  it("covers the band, rounding out to whole rows", () => {
    expect(rowRange(tops, 10, 0, 250)).toEqual([0, 3]);
    expect(rowRange(tops, 10, 150, 450)).toEqual([1, 5]);
  });

  it("stays inside the list", () => {
    expect(rowRange(tops, 10, -500, 50)).toEqual([0, 1]);
    expect(rowRange(tops, 10, 900, 5000)).toEqual([9, 10]);
    expect(rowRange(tops, 10, 5000, 6000)).toEqual([9, 10]);
    expect(rowRange(tops, 10, -900, -100)).toEqual([0, 1]);
    expect(rowRange(tops, 0, 0, 100)).toEqual([0, 0]);
  });
});
