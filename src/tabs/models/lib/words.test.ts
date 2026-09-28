import { describe, expect, it } from "vitest";
import {
  backendPlain,
  baseName,
  formatLabel,
  groupFraction,
  isModelFile,
  lastUsedText,
  progressText,
  quantPlain,
  ratioPercent,
  sniffImageType,
  tierPlain,
} from "./words";

describe("lastUsedText", () => {
  const now = Date.UTC(2026, 8, 28, 12, 0, 0);
  const min = 60_000;
  const day = 24 * 60 * min;
  it("handles never", () => {
    expect(lastUsedText(null, now)).toBe("Never");
    expect(lastUsedText(0, now)).toBe("Never");
  });
  it("buckets recent times", () => {
    expect(lastUsedText(now - 30_000, now)).toBe("Just now");
    expect(lastUsedText(now - 15 * min, now)).toBe("15 min ago");
    expect(lastUsedText(now - 5 * 60 * min, now)).toBe("5 h ago");
    expect(lastUsedText(now - 1.5 * day, now)).toBe("Yesterday");
    expect(lastUsedText(now - 6 * day, now)).toBe("6 days ago");
    expect(lastUsedText(now - 70 * day, now)).toBe("2 months ago");
    expect(lastUsedText(now - 400 * day, now)).toBe("Over a year ago");
  });
  it("accepts unix seconds", () => {
    expect(lastUsedText(Math.floor((now - 15 * min) / 1000), now)).toBe("15 min ago");
  });
});

describe("download progress", () => {
  it("computes the fraction and text", () => {
    const g = { downloadedBytes: 1024 ** 3, totalBytes: 4 * 1024 ** 3, state: "downloading" as const };
    expect(groupFraction(g)).toBe(0.25);
    expect(progressText(g)).toBe("1.0 GB of 4.0 GB · 25%");
  });
  it("is full while verifying and safe with unknown totals", () => {
    expect(groupFraction({ downloadedBytes: 5, totalBytes: 10, state: "verifying" })).toBe(1);
    expect(groupFraction({ downloadedBytes: 5, totalBytes: 0, state: "downloading" })).toBe(0);
    expect(progressText({ downloadedBytes: 0, totalBytes: 0, state: "queued" })).toBe("Starting…");
  });
});

describe("plain words", () => {
  it("describes backends and tiers", () => {
    expect(backendPlain("cuda")).toMatch(/NVIDIA/);
    expect(backendPlain("cpu")).toMatch(/slow/);
    expect(tierPlain("high")).toBe("High-end (13–20 GB)");
  });
  it("describes quants", () => {
    expect(quantPlain("bf16")).toBe("Full quality version");
    expect(quantPlain("q8_0")).toBe("High-quality compact version");
    expect(quantPlain("q4_k")).toBe("Smaller version for your GPU");
    expect(quantPlain(null)).toBeNull();
  });
  it("formats ratios and formats", () => {
    expect(ratioPercent(0.934)).toBe("93%");
    expect(ratioPercent(88)).toBe("88%");
    expect(ratioPercent(null)).toBeNull();
    expect(formatLabel("SafeTensor")).toBe("SafeTensors");
    expect(formatLabel("gguf")).toBe("GGUF");
  });
});

describe("files", () => {
  it("gets base names on both platforms", () => {
    expect(baseName("C:\\Users\\me\\Downloads\\model.safetensors")).toBe("model.safetensors");
    expect(baseName("/home/me/model.gguf")).toBe("model.gguf");
  });
  it("accepts only safetensors and gguf", () => {
    expect(isModelFile("a.SafeTensors")).toBe(true);
    expect(isModelFile("a.gguf")).toBe(true);
    expect(isModelFile("a.ckpt")).toBe(false);
    expect(isModelFile("a.pt")).toBe(false);
  });
  it("sniffs image types", () => {
    expect(sniffImageType(new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]))).toBe("image/png");
    expect(sniffImageType(new Uint8Array([0xff, 0xd8, 0xff, 0xe0]))).toBe("image/jpeg");
    expect(sniffImageType(new Uint8Array([0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57, 0x45, 0x42, 0x50]))).toBe("image/webp");
    expect(sniffImageType(new Uint8Array([1, 2, 3]))).toBe("application/octet-stream");
  });
});
