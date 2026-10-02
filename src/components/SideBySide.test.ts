import { describe, expect, it } from "vitest";
import { sideLayout } from "./SideBySide";

describe("sideLayout", () => {
  it("puts two squares in a row in a wide space, at the same height", () => {
    const l = sideLayout(1000, 400, { width: 512, height: 512 }, { width: 1024, height: 1024 });
    expect(l.direction).toBe("row");
    expect(l.first).toEqual({ width: 400, height: 400 });
    expect(l.second).toEqual({ width: 400, height: 400 });
  });

  it("stacks two wide pictures in a tall space, at the same width", () => {
    const l = sideLayout(400, 1000, { width: 1600, height: 900 }, { width: 1600, height: 900 });
    expect(l.direction).toBe("column");
    expect(l.first.width).toBe(400);
    expect(l.first.height + l.second.height).toBeLessThanOrEqual(1000);
  });

  it("fits pictures of different shapes and draws the larger one at most twice its size", () => {
    const l = sideLayout(2000, 2000, { width: 100, height: 200 }, { width: 300, height: 200 });
    expect(l.second).toEqual({ width: 600, height: 400 });
  });

  it("a small reference doesn't shrink a large result", () => {
    const l = sideLayout(1000, 700, { width: 128, height: 128 }, { width: 1024, height: 1024 });
    expect(l.second.height).toBe(494);
  });
});
