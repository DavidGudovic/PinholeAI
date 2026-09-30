import { describe, expect, it } from "vitest";
import type { CatalogCard } from "../../../lib/types";
import {
  addTotals,
  changedFilterCount,
  forModelOf,
  defaultFilters,
  FALLBACK_OPTIONS,
  filtersKey,
  isSafeModeOff,
  isVideoFile,
  mergePage,
  NO_TOTALS,
  normalizeSearch,
  resultsSummary,
  shouldBlurPreview,
  showPriceBadge,
  tagsWithSafeMode,
  toBrowseQuery,
  toggleTag,
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
    expect(f).toMatchObject({ kind: "models", look: null, tags: [], content: "safe", price: "free", compatibleOnly: true, commercialOnly: false });
    // Mainstream models first: "This month · Top rated" was mostly fresh anime merges.
    expect(f.sort).toBe("Most Downloaded");
    expect(f.period).toBe("AllTime");
  });

  it("includes paid models when Settings → Show paid is on", () => {
    expect(defaultFilters(FALLBACK_OPTIONS, { contentMode: "safe", showPaid: true }, false).price).toBe("include");
  });

  it("starts with Hide anime as remembered in Settings", () => {
    expect(defaultFilters(FALLBACK_OPTIONS, { contentMode: "safe", showPaid: false }, false).hideAnime).toBe(false);
    const on = defaultFilters(FALLBACK_OPTIONS, { contentMode: "safe", showPaid: false, hideAnime: true }, false);
    expect(on.hideAnime).toBe(true);
    expect(toBrowseQuery(on).hideAnime).toBe(true);
    expect(filtersKey(on)).not.toBe(filtersKey({ ...on, hideAnime: false }));
  });

  it("only applies a Safe mode Off default after this session's confirmation", () => {
    const s = { contentMode: "all" as const, showPaid: false };
    expect(defaultFilters(FALLBACK_OPTIONS, s, false).content).toBe("safe");
    expect(defaultFilters(FALLBACK_OPTIONS, s, true).content).toBe("all");
  });
});

describe("toBrowseQuery", () => {
  it("maps every filter and normalises the search text", () => {
    const f = { ...defaultFilters(null, null, false), look: "anime", tags: ["fantasy", "edit", "fantasy"], kind: "styleAddons" as const, query: "  pixel   art \n" };
    expect(toBrowseQuery(f, "abc")).toEqual({
      kind: "styleAddons",
      look: "anime",
      tags: ["edit", "fantasy"],
      content: "safe",
      price: "free",
      sort: "Most Downloaded",
      period: "AllTime",
      commercialOnly: false,
      compatibleOnly: true,
      runsOnMyCard: false,
      hideAnime: false,
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
    expect(filtersKey({ ...base, tags: ["edit"] })).not.toBe(filtersKey(base));
    expect(filtersKey({ ...base, runsOnMyCard: true })).not.toBe(filtersKey(base));
  });
  it("ignores the order tags were picked in", () => {
    expect(filtersKey({ ...base, tags: ["edit", "animals"] })).toBe(filtersKey({ ...base, tags: ["animals", "edit"] }));
  });
  it("sends “Runs on my card” for models only", () => {
    expect(toBrowseQuery({ ...base, runsOnMyCard: true }).runsOnMyCard).toBe(true);
    expect(toBrowseQuery({ ...base, kind: "styleAddons", runsOnMyCard: true }).runsOnMyCard).toBe(false);
  });
  it("counts changed filters", () => {
    expect(changedFilterCount(base, base)).toBe(0);
    expect(changedFilterCount({ ...base, look: "anime", query: "x", compatibleOnly: false }, base)).toBe(3);
    expect(changedFilterCount({ ...base, tags: ["edit", "animals"] }, base)).toBe(1);
    expect(changedFilterCount({ ...base, runsOnMyCard: true }, base)).toBe(1);
    expect(changedFilterCount({ ...base, kind: "styleAddons", runsOnMyCard: true }, { ...base, kind: "styleAddons" })).toBe(0);
  });
  it("narrows style add-ons to one model, and only style add-ons", () => {
    const addons = { ...base, kind: "styleAddons" as const };
    expect(forModelOf({ ...addons, forModel: "juggernaut" })).toBe("juggernaut");
    expect(forModelOf({ ...base, forModel: "juggernaut" })).toBeNull();
    expect(filtersKey({ ...addons, forModel: "juggernaut" })).not.toBe(filtersKey(addons));
    expect(filtersKey({ ...base, forModel: "juggernaut" })).toBe(filtersKey(base));
    // Picking the model isn't a filter to clear: it stays like the kind does.
    expect(changedFilterCount({ ...addons, forModel: "juggernaut" }, addons)).toBe(0);
  });
});

describe("content helpers", () => {
  it("knows when Safe mode is off", () => {
    expect(isSafeModeOff("safe")).toBe(false);
    expect(isSafeModeOff("all")).toBe(true);
  });
  it("blurs NSFW previews only while Safe mode is on", () => {
    expect(shouldBlurPreview(card(1, { previewNsfw: true }), "safe")).toBe(true);
    expect(shouldBlurPreview(card(1, { previewNsfw: true }), "all")).toBe(false);
    expect(shouldBlurPreview(card(1), "safe")).toBe(false);
  });
  it("shows price badges only when paid models can appear", () => {
    expect(showPriceBadge("free")).toBe(false);
    expect(showPriceBadge("include")).toBe(true);
    expect(showPriceBadge("paid_only")).toBe(true);
  });
});

describe("tags", () => {
  it("picks and unpicks", () => {
    expect(toggleTag([], "edit")).toEqual(["edit"]);
    expect(toggleTag(["edit", "nsfw"], "edit")).toEqual(["nsfw"]);
  });
  it("drops the NSFW tag when Safe mode turns on", () => {
    expect(tagsWithSafeMode(["edit", "nsfw", "fantasy"], FALLBACK_OPTIONS)).toEqual(["edit", "fantasy"]);
  });
  it("has no one-click NSFW preset: NSFW is only a tag", () => {
    expect(FALLBACK_OPTIONS.content.map((c) => c.key)).toEqual(["safe", "all"]);
    expect(FALLBACK_OPTIONS.tags.filter((t) => t.needsSafeModeOff).map((t) => t.key)).toEqual(["nsfw"]);
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
    expect(resultsSummary({ kind: "models", content: "all", compatibleOnly: false, runsOnMyCard: true }, 5, page(50, 0, 0, 7)).hints).toEqual([
      "“Runs on my card” hid 7 too big for your graphics card.",
    ]);
    expect(resultsSummary({ kind: "models", content: "all", compatibleOnly: false, runsOnMyCard: false }, 5, page(50, 0, 0, 7)).hints).toEqual([]);
  });
  it("says why models are missing and what to change", () => {
    const s = resultsSummary({ kind: "models", content: "safe", compatibleOnly: true }, 24, page(50, 20, 6));
    expect(s.count).toBe("24 models");
    expect(s.hints).toEqual(["Showing models that run in Pinhole — turn off “Works with Pinhole” to see all.", "Safe mode hid 20 made for adults."]);
  });
  it("names style add-ons and drops hints that don't apply", () => {
    expect(resultsSummary({ kind: "styleAddons", content: "safe", compatibleOnly: true }, 1, NO_TOTALS)).toEqual({
      count: "1 style add-on",
      hints: ["Showing style add-ons that work in Pinhole — turn off “Works with Pinhole” to see all."],
    });
    expect(resultsSummary({ kind: "models", content: "all", compatibleOnly: false }, 1200, page(50, 3, 0))).toEqual({ count: "1,200 models", hints: [] });
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
