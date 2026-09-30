import { describe, expect, it } from "vitest";
import { recallStep, shouldRecall } from "./promptRecall";

const h = ["first", "second", "third"];

describe("recallStep", () => {
  it("Up walks back from the newest and keeps the draft", () => {
    const a = recallStep(h, null, "my draft", -1);
    expect(a.text).toBe("third");
    const b = recallStep(h, a.browse, "third", -1);
    expect(b.text).toBe("second");
    expect(b.browse?.draft).toBe("my draft");
    const c = recallStep(h, recallStep(h, b.browse, "second", -1).browse, "first", -1);
    expect(c.text).toBe("first"); // stays at the oldest
  });
  it("Down walks forward and finally restores the draft", () => {
    const up2 = recallStep(h, recallStep(h, null, "draft", -1).browse, "third", -1);
    const d1 = recallStep(h, up2.browse, "second", 1);
    expect(d1.text).toBe("third");
    const d2 = recallStep(h, d1.browse, "third", 1);
    expect(d2).toEqual({ browse: null, text: "draft" });
  });
  it("does nothing without history, or on Down when not browsing", () => {
    expect(recallStep([], null, "x", -1).text).toBeNull();
    expect(recallStep(h, null, "x", 1).text).toBeNull();
  });
});

describe("shouldRecall", () => {
  it("Up recalls only at the very start (or while browsing) of the first line", () => {
    expect(shouldRecall("", 0, 0, -1, false)).toBe(true);
    expect(shouldRecall("hello", 0, 0, -1, false)).toBe(true);
    expect(shouldRecall("hello", 3, 3, -1, false)).toBe(false);
    expect(shouldRecall("hello", 5, 5, -1, true)).toBe(true);
    expect(shouldRecall("a\nb", 3, 3, -1, true)).toBe(false);
    expect(shouldRecall("hello", 0, 3, -1, false)).toBe(false);
  });
  it("Down only while browsing and on the last line", () => {
    expect(shouldRecall("hello", 5, 5, 1, false)).toBe(false);
    expect(shouldRecall("hello", 5, 5, 1, true)).toBe(true);
    expect(shouldRecall("a\nb", 0, 0, 1, true)).toBe(false);
  });
});
