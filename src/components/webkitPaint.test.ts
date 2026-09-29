/// <reference types="vite/client" />
// On Linux the WebView is WebKitGTK. With NVIDIA drivers it repaints `backdrop-filter`
// on every scroll frame (laggy Models list and details page) and draws an image that
// animates a transform inside a CSS column layout as a gray box on hover. Keep both out
// of the UI: use solid backgrounds, and a ring (not a zoom) for hover on images.
import { describe, expect, it } from "vitest";

const sources = import.meta.glob(["../**/*.tsx", "../**/*.css", "!../**/*.test.tsx"], { query: "?raw", import: "default", eager: true }) as Record<string, string>;

describe("WebKitGTK-friendly styles", () => {
  it("finds the UI sources", () => {
    expect(Object.keys(sources).length).toBeGreaterThan(10);
  });

  it("uses no backdrop blur", () => {
    const hits = Object.entries(sources).filter(([, text]) => /backdrop-blur|backdrop-filter/.test(text)).map(([file]) => file);
    expect(hits).toEqual([]);
  });

  it("doesn't zoom images on hover", () => {
    const hits = Object.entries(sources).filter(([, text]) => /(group-)?hover:scale-/.test(text)).map(([file]) => file);
    expect(hits).toEqual([]);
  });
});
