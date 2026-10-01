import { describe, expect, it } from "vitest";
import { isWebKitGtk, markPlatform, WEBKITGTK_CSS } from "./platform";

const LINUX = "Mozilla/5.0 (X11; Ubuntu; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko)";
const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 Edg/140.0.0.0";

describe("platform", () => {
  it("detects only the Linux WebView", () => {
    expect(isWebKitGtk(LINUX)).toBe(true);
    expect(isWebKitGtk(WINDOWS)).toBe(false);
    expect(isWebKitGtk("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36")).toBe(false);
    expect(isWebKitGtk("Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0")).toBe(false);
  });

  it("turns off backdrop blur, hover zoom and Browse card shadows on Linux", () => {
    const doc = document.implementation.createHTMLDocument();
    markPlatform(doc, LINUX);
    expect(doc.documentElement.classList.contains("webkitgtk")).toBe(true);
    expect(doc.head.querySelector("style")?.textContent).toBe(WEBKITGTK_CSS);
    expect(WEBKITGTK_CSS).toMatch(/backdrop-filter: none/);
    expect(WEBKITGTK_CSS).toMatch(/\.pinhole-hover-zoom \{[^}]*scale: none/);
    expect(WEBKITGTK_CSS).toMatch(/\.pinhole-card \{[^}]*box-shadow: none/);
    expect(WEBKITGTK_CSS).toMatch(/\.pinhole-card \.animate-pulse \{[^}]*animation: none/);
  });

  it("leaves Windows alone", () => {
    const doc = document.implementation.createHTMLDocument();
    markPlatform(doc, WINDOWS);
    expect(doc.documentElement.classList.contains("webkitgtk")).toBe(false);
    expect(doc.head.querySelector("style")).toBeNull();
  });
});
