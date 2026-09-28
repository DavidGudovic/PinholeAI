import { describe, expect, it } from "vitest";
import { imageMime } from "./images";

const buf = (...bytes: number[]) => new Uint8Array([...bytes, ...new Array(16).fill(0)]).buffer;

describe("imageMime", () => {
  it("types session bytes by their magic number (imported images keep their format)", () => {
    expect(imageMime(buf(0x89, 0x50, 0x4e, 0x47))).toBe("image/png");
    expect(imageMime(buf(0xff, 0xd8, 0xff, 0xe0))).toBe("image/jpeg");
    expect(imageMime(buf(0x52, 0x49, 0x46, 0x46, 1, 2, 3, 4, 0x57, 0x45, 0x42, 0x50))).toBe("image/webp");
    expect(imageMime(new ArrayBuffer(0))).toBe("image/png");
  });
});
