import { describe, expect, it } from "vitest";
import type { CatalogCard } from "../../../lib/types";
import {
  changedFilterCount,
  defaultFilters,
  FALLBACK_OPTIONS,
  filtersKey,
  isAdult,
  mergePage,
  normalizeSearch,
  shouldBlurPreview,
  showPriceBadge,
  toBrowseQuery,
} from "./query";

const card = (versionId: number, extra: Partial<CatalogCard> = {}): CatalogCard => ({
  modelId: versionId * 10,
  versionId,
  name: `M${versionId}`,
  versionName: "v1",
  type: "Checkpoint",
  baseModel: "SDXL 1.0",
  familyId: "sdxl",
  styleBadge: null,
  creator: null,
  previewUrl: null,
  previewIsVideo: false,
  previewNsfw: false,
  modelNsfw: false,
  thumbsUpRatio: 0.9,
  downloadCount: 1,
  downloadBytes: 1,
  vram: null,
  fit: null,
  earlyAccess: false,
  commercialOk: true,
  licenseNote: null,
  installed: false,
  blockedReason: null,
  ...extra,
});

describe("defaultFilters", () => {
  it("uses safe + free + compatible by default", () => {
    const f = defaultFilters(null, null, false);
    expect(f).toMatchObject({ kind: "models", look: null, content: "safe", price: "free", compatibleOnly: true, commercialOnly: false });
    expect(f.sort).toBe("Highest Rated");
    expect(f.period).toBe("Month");
  });

  it("includes paid models when Settings → Show paid is on", () => {
    expect(defaultFilters(FALLBACK_OPTIONS, { contentMode: "safe", showPaid: true }, false).price).toBe("include");
  });

  it("only applies an 18+ default after this session's confirmation", () => {
    const s = { contentMode: "only_18plus" as const, showPaid: false };
    expect(defaultFilters(FALLBACK_OPTIONS, s, false).content).toBe("safe");
    expect(defaultFilters(FALLBACK_OPTIONS, s, true).content).toBe("only_18plus");
  });
});

describe("toBrowseQuery", () => {
  it("maps every filter and normalises the search text", () => {
    const f = { ...defaultFilters(null, null, false), look: "anime", kind: "styleAddons" as const, query: "  pixel   art \n" };
    expect(toBrowseQuery(f, "abc")).toEqual({
      kind: "styleAddons",
      look: "anime",
      content: "safe",
      price: "free",
      sort: "Highest Rated",
      period: "Month",
      commercialOnly: false,
      compatibleOnly: true,
      query: "pixel art",
      cursor: "abc",
    });
  });

  it("defaults the cursor to null (first page)", () => {
    expect(toBrowseQuery(defaultFilters(null, null, false)).cursor).toBeNull();
  });

  it("caps very long searches", () => {
    expect(normalizeSearch("x".repeat(500))).toHaveLength(200);
  });
});

describe("filtersKey / changedFilterCount", () => {
  const base = defaultFilters(null, null, false);
  it("ignores whitespace-only search differences", () => {
    expect(filtersKey({ ...base, query: " a  b " })).toBe(filtersKey({ ...base, query: "a b" }));
  });
  it("changes when a filter changes", () => {
    expect(filtersKey({ ...base, commercialOnly: true })).not.toBe(filtersKey(base));
  });
  it("counts changed filters", () => {
    expect(changedFilterCount(base, base)).toBe(0);
    expect(changedFilterCount({ ...base, look: "anime", query: "x", compatibleOnly: false }, base)).toBe(3);
  });
});

describe("content helpers", () => {
  it("knows which modes are 18+", () => {
    expect(isAdult("safe")).toBe(false);
    expect(isAdult("include_18plus")).toBe(true);
    expect(isAdult("only_18plus")).toBe(true);
  });
  it("blurs NSFW previews only when 18+ is off", () => {
    expect(shouldBlurPreview(card(1, { previewNsfw: true }), "safe")).toBe(true);
    expect(shouldBlurPreview(card(1, { previewNsfw: true }), "include_18plus")).toBe(false);
    expect(shouldBlurPreview(card(1), "safe")).toBe(false);
  });
  it("shows price badges only when paid models can appear", () => {
    expect(showPriceBadge("free")).toBe(false);
    expect(showPriceBadge("include")).toBe(true);
    expect(showPriceBadge("paid_only")).toBe(true);
  });
});

describe("mergePage", () => {
  it("appends new cards and drops repeats", () => {
    const merged = mergePage([card(1), card(2)], [card(2), card(3)]);
    expect(merged.map((c) => c.versionId)).toEqual([1, 2, 3]);
  });
});
