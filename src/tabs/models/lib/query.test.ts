import { describe, expect, it } from "vitest";
import type { CatalogCard } from "../../../lib/types";
import {
  addTotals,
  changedFilterCount,
  defaultFilters,
  FALLBACK_OPTIONS,
  filtersKey,
  isAdult,
  isVideoFile,
  mergePage,
  NO_TOTALS,
  normalizeSearch,
  resultsSummary,
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
    // Mainstream models first: "This month · Top rated" was mostly fresh anime merges.
    expect(f.sort).toBe("Most Downloaded");
    expect(f.period).toBe("AllTime");
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
      sort: "Most Downloaded",
      period: "AllTime",
      commercialOnly: false,
      compatibleOnly: true,
      runsOnMyCard: false,
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
    expect(filtersKey({ ...base, runsOnMyCard: true })).not.toBe(filtersKey(base));
  });
  it("sends “Runs on my card” for models only", () => {
    expect(toBrowseQuery({ ...base, runsOnMyCard: true }).runsOnMyCard).toBe(true);
    expect(toBrowseQuery({ ...base, kind: "styleAddons", runsOnMyCard: true }).runsOnMyCard).toBe(false);
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

describe("totals and the line above the grid", () => {
  const page = (checked: number, hiddenByContent: number, hiddenByFilters: number, hiddenBySize = 0) => ({ checked, hiddenByContent, hiddenByFilters, hiddenBySize });
  it("adds page counts", () => {
    expect(addTotals(addTotals(NO_TOTALS, page(50, 20, 6, 2)), page(50, 10, 4))).toEqual({ checked: 100, hiddenByContent: 30, hiddenByFilters: 10, hiddenBySize: 2 });
  });
  it("says how many “Runs on my card” hid, for models only", () => {
    expect(resultsSummary({ kind: "models", content: "include_18plus", compatibleOnly: false, runsOnMyCard: true }, 5, page(50, 0, 0, 7)).hints).toEqual([
      "“Runs on my card” hid 7 too big for your graphics card.",
    ]);
    expect(resultsSummary({ kind: "models", content: "include_18plus", compatibleOnly: false, runsOnMyCard: false }, 5, page(50, 0, 0, 7)).hints).toEqual([]);
  });
  it("says why models are missing and what to change", () => {
    const s = resultsSummary({ kind: "models", content: "safe", compatibleOnly: true }, 24, page(50, 20, 6));
    expect(s.count).toBe("24 models");
    expect(s.hints).toEqual(["Showing models that run in Pinhole — turn off “Works with Pinhole” to see all.", "“Safe only” hid 20 made for adults."]);
  });
  it("names style add-ons and drops hints that don't apply", () => {
    expect(resultsSummary({ kind: "styleAddons", content: "safe", compatibleOnly: true }, 1, NO_TOTALS)).toEqual({
      count: "1 style add-on",
      hints: ["Showing style add-ons that work in Pinhole — turn off “Works with Pinhole” to see all."],
    });
    expect(resultsSummary({ kind: "models", content: "include_18plus", compatibleOnly: false }, 1200, page(50, 3, 0))).toEqual({ count: "1,200 models", hints: [] });
  });
});

describe("isVideoFile", () => {
  it("fetches stills of videos, never the video itself", () => {
    expect(isVideoFile("https://image.civitai.com/x/y/anim=false,transcode=true,width=450,optimized=true/1.jpeg")).toBe(false);
    expect(isVideoFile("https://image.civitai.com/x/v.mp4")).toBe(true);
    expect(isVideoFile("https://image.civitai.com/x/v.WEBM?x=1")).toBe(true);
    expect(isVideoFile(null)).toBe(false);
  });
});
