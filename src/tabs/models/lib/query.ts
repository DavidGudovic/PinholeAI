// Pure helpers for the Models → Browse filter bar (SPEC §5.4).
// The UI keeps a `BrowseFilters` object; `toBrowseQuery` turns it into the IPC
// `BrowseQuery`. The CivitAI mapping itself (types=, nsfw=, baseModels=…) lives in Rust.
import type { BrowsePage, BrowseQuery, CatalogCard, CatalogFilterOptions, CatalogKind, ContentMode, PriceMode, Settings } from "../../../lib/types";

export interface BrowseFilters {
  kind: CatalogKind;
  look: string | null;
  /** Picked tag keys, in the order they were picked. */
  tags: string[];
  content: ContentMode;
  price: PriceMode;
  sort: string;
  period: string;
  commercialOnly: boolean;
  compatibleOnly: boolean;
  /** Hide models that are too big for this computer's graphics card (models only). */
  runsOnMyCard: boolean;
  /** Hide anime models and add-ons (remembered in Settings). */
  hideAnime: boolean;
  /** Style add-ons: only ones made for this installed model (its id), or any. */
  forModel: string | null;
  query: string;
}

/** Fallback option lists (used until `catalog_filters` answers, and in tests). Labels mirror catalog-filters.yaml. */
export const FALLBACK_OPTIONS: CatalogFilterOptions = {
  looks: [
    { key: "realistic", label: "Realistic" },
    { key: "anime", label: "Anime" },
    { key: "illustration", label: "Illustration" },
    { key: "three_d", label: "3D" },
    { key: "painting", label: "Painting" },
    { key: "pixel_art", label: "Pixel art" },
    { key: "line_art", label: "Line art" },
    { key: "cinematic", label: "Cinematic" },
    { key: "vintage", label: "Vintage" },
    { key: "brand", label: "Brand & product" },
  ],
  tags: [
    { key: "edit", label: "Edit model", needsSafeModeOff: false },
    { key: "reference", label: "Reference picture", needsSafeModeOff: false },
    { key: "portraits", label: "Portraits", needsSafeModeOff: false },
    { key: "characters", label: "Characters", needsSafeModeOff: false },
    { key: "landscapes", label: "Landscapes", needsSafeModeOff: false },
    { key: "architecture", label: "Architecture", needsSafeModeOff: false },
    { key: "animals", label: "Animals", needsSafeModeOff: false },
    { key: "fantasy", label: "Fantasy", needsSafeModeOff: false },
    { key: "scifi", label: "Sci-fi", needsSafeModeOff: false },
    { key: "vehicles", label: "Vehicles", needsSafeModeOff: false },
    { key: "robots", label: "Robots", needsSafeModeOff: false },
    { key: "food", label: "Food", needsSafeModeOff: false },
    { key: "fashion", label: "Fashion", needsSafeModeOff: false },
    { key: "objects", label: "Objects", needsSafeModeOff: false },
    { key: "backgrounds", label: "Backgrounds", needsSafeModeOff: false },
    { key: "textures", label: "Textures", needsSafeModeOff: false },
    { key: "horror", label: "Horror", needsSafeModeOff: false },
    { key: "nsfw", label: "NSFW", needsSafeModeOff: true },
  ],
  sorts: [
    { label: "Most liked", api: "Most Liked" },
    { label: "Most downloaded", api: "Most Downloaded" },
    { label: "Newest", api: "Newest" },
  ],
  periods: [
    { label: "This week", api: "Week" },
    { label: "This month", api: "Month" },
    { label: "This year", api: "Year" },
    { label: "All time", api: "AllTime" },
  ],
  content: [
    { key: "safe", label: "On" },
    { key: "all", label: "Off" },
  ],
  price: [
    { key: "free", label: "Free" },
    { key: "include", label: "Include early access (paid)" },
    { key: "paid_only", label: "Early access only" },
  ],
  defaultContent: "safe",
  defaultPrice: "free",
  defaultSort: "Most Downloaded",
  defaultPeriod: "AllTime",
};

export const KIND_OPTIONS: { value: CatalogKind; label: string }[] = [
  { value: "models", label: "Models" },
  { value: "styleAddons", label: "Style add-ons" },
];

export const COMMERCIAL_OPTIONS: { value: "any" | "ok"; label: string }[] = [
  { value: "any", label: "Any" },
  { value: "ok", label: "OK for client work" },
];

/** Safe mode is off (turning it off asks to confirm once per session). */
export function isSafeModeOff(mode: ContentMode): boolean {
  return mode !== "safe";
}

/** "Off" only applies once the user confirmed this session; until then Safe mode stays on. */
export function confirmedContent(wanted: ContentMode, adultConfirmed: boolean): ContentMode {
  return isSafeModeOff(wanted) && !adultConfirmed ? "safe" : wanted;
}

/**
 * Safe mode for a model's example pictures opened outside Browse (Installed). Off only when
 * Settings has it off, the user confirmed this session, and Browse (once opened) has it off too.
 */
export function examplesContent(settingsMode: ContentMode | null, adultConfirmed: boolean, browse: Pick<BrowseFilters, "content"> | null): ContentMode {
  const fromSettings = confirmedContent(settingsMode ?? "safe", adultConfirmed);
  return browse && !isSafeModeOff(browse.content) ? "safe" : fromSettings;
}

/**
 * Initial filters. Safe mode: the Settings default, but "Off" only applies once the user
 * confirmed this session (otherwise start with Safe mode on and ask). Price: "Include early
 * access" when Settings → Show paid is on, else the catalog default (Free).
 */
export function defaultFilters(
  options: CatalogFilterOptions | null,
  settings: Pick<Settings, "contentMode" | "showPaid"> & Partial<Pick<Settings, "hideAnime">> | null,
  adultConfirmed: boolean,
): BrowseFilters {
  const o = options ?? FALLBACK_OPTIONS;
  const wantedContent = settings?.contentMode ?? o.defaultContent;
  const content = confirmedContent(wantedContent, adultConfirmed);
  const price: PriceMode = settings?.showPaid ? "include" : o.defaultPrice;
  return {
    kind: "models",
    look: null,
    tags: [],
    content,
    price,
    sort: pickOption(o.sorts, o.defaultSort) ?? "Most Downloaded",
    period: pickOption(o.periods, o.defaultPeriod) ?? "AllTime",
    commercialOnly: false,
    compatibleOnly: true,
    runsOnMyCard: false,
    hideAnime: settings?.hideAnime ?? false,
    forModel: null,
    query: "",
  };
}

/** `wanted` when it is one of the options, else the first option. */
function pickOption(list: { api: string }[], wanted: string | undefined): string | undefined {
  return list.find((o) => o.api === wanted)?.api ?? list[0]?.api;
}

/** Tags as sent: sorted and unique (picking order doesn't change the results or the cache key). */
export function tagsFor(f: Pick<BrowseFilters, "tags">): string[] {
  return [...new Set(f.tags)].sort();
}

/** The Reference picture tag: shown as its own switch, not with the other tags. */
export const REFERENCE_TAG = "reference";

/**
 * Filters set under "More filters" (tags other than Reference picture, Price, Commercial use,
 * Hide anime): the count on the closed toggle, and whether it opens.
 */
export function moreFiltersCount(f: BrowseFilters, defaults: Pick<BrowseFilters, "price">): number {
  const tags = tagsFor(f).filter((t) => t !== REFERENCE_TAG).length;
  return tags + (f.price !== defaults.price ? 1 : 0) + (f.commercialOnly ? 1 : 0) + (f.hideAnime ? 1 : 0);
}

/** Pick or unpick one tag. */
export function toggleTag(tags: string[], key: string): string[] {
  return tags.includes(key) ? tags.filter((t) => t !== key) : [...tags, key];
}

/** Tags that stay picked when Safe mode turns on (the NSFW tag can't find anything then). */
export function tagsWithSafeMode(tags: string[], options: Pick<CatalogFilterOptions, "tags">): string[] {
  const offOnly = new Set(options.tags.filter((t) => t.needsSafeModeOff).map((t) => t.key));
  return tags.filter((t) => !offOnly.has(t));
}

/** Tags Browse shows: the ones that only work with Safe mode off are left out while it is on. */
export function visibleTags<T extends { needsSafeModeOff?: boolean }>(tags: T[], content: ContentMode): T[] {
  return isSafeModeOff(content) ? tags : tags.filter((t) => !t.needsSafeModeOff);
}

/** Normalise free text the way we send it: trimmed, inner whitespace collapsed, capped. */
export function normalizeSearch(q: string): string {
  return q.replace(/\s+/g, " ").trim().slice(0, 200);
}

export function toBrowseQuery(f: BrowseFilters, cursor: string | null = null): BrowseQuery {
  return {
    kind: f.kind,
    look: f.look,
    tags: tagsFor(f),
    content: f.content,
    price: f.price,
    sort: f.sort,
    period: f.period,
    commercialOnly: f.commercialOnly,
    compatibleOnly: f.compatibleOnly,
    runsOnMyCard: f.kind === "models" && f.runsOnMyCard,
    hideAnime: f.hideAnime,
    query: normalizeSearch(f.query),
    cursor,
  };
}

/** Stable key: a new key means "start over from the first page". */
export function filtersKey(f: BrowseFilters): string {
  const q = toBrowseQuery(f, null);
  return JSON.stringify([q.kind, q.look, q.tags, q.content, q.price, q.sort, q.period, q.commercialOnly, q.compatibleOnly, q.runsOnMyCard, q.hideAnime, q.query, forModelOf(f)]);
}

/** The installed model style add-ons are narrowed to (only while browsing add-ons). */
export function forModelOf(f: Pick<BrowseFilters, "kind" | "forModel">): string | null {
  return f.kind === "styleAddons" ? f.forModel : null;
}

/** Filters that differ from the defaults (for a "Clear filters" button). */
export function changedFilterCount(f: BrowseFilters, defaults: BrowseFilters): number {
  const keys: (keyof BrowseFilters)[] = ["look", "content", "price", "sort", "period", "commercialOnly", "compatibleOnly", "runsOnMyCard", "hideAnime", "query"];
  // "Runs on my card" is hidden (and not sent) for style add-ons.
  const shown = f.kind === "models" ? keys : keys.filter((k) => k !== "runsOnMyCard");
  const tagsChanged = tagsFor(f).join() !== tagsFor(defaults).join() ? 1 : 0;
  return tagsChanged + shown.filter((k) => (k === "query" ? normalizeSearch(f.query) !== normalizeSearch(defaults.query) : f[k] !== defaults[k])).length;
}

/** Price badges are only shown when paid models can appear (SPEC §5.4 model card). */
export function showPriceBadge(price: PriceMode): boolean {
  return price !== "free";
}

/** Blur previews flagged NSFW while Safe mode is on (catalog-filters.yaml: blur_nsfw_previews_when_safe). */
export function shouldBlurPreview(card: Pick<CatalogCard, "previewNsfw" | "modelNsfw">, content: ContentMode): boolean {
  return !isSafeModeOff(content) && (card.previewNsfw || card.modelNsfw);
}

/** A preview URL that points at a video file (not a still frame of it): never fetched. */
export function isVideoFile(url: string | null): boolean {
  return !!url && /\.(mp4|webm|mov)$/i.test(url.split(/[?#]/)[0]);
}

/** Append a page, dropping duplicates (the API can repeat items across cursor pages). */
export function mergePage(existing: CatalogCard[], incoming: CatalogCard[]): CatalogCard[] {
  const seen = new Set(existing.map((c) => c.versionId));
  const out = existing.slice();
  for (const c of incoming) {
    if (!seen.has(c.versionId)) {
      seen.add(c.versionId);
      out.push(c);
    }
  }
  return out;
}

/** Running totals over the loaded pages (for the line above the grid). */
export interface BrowseTotals {
  checked: number;
  hiddenByContent: number;
  hiddenByFilters: number;
  hiddenBySize: number;
}

export const NO_TOTALS: BrowseTotals = { checked: 0, hiddenByContent: 0, hiddenByFilters: 0, hiddenBySize: 0 };

export function addTotals(a: BrowseTotals, page: Pick<BrowsePage, "checked" | "hiddenByContent" | "hiddenByFilters" | "hiddenBySize">): BrowseTotals {
  return {
    checked: a.checked + (page.checked ?? 0),
    hiddenByContent: a.hiddenByContent + (page.hiddenByContent ?? 0),
    hiddenByFilters: a.hiddenByFilters + (page.hiddenByFilters ?? 0),
    hiddenBySize: a.hiddenBySize + (page.hiddenBySize ?? 0),
  };
}

/**
 * The plain-words line above the grid: how many cards, and why some models aren't there
 * ("Works with Pinhole", Safe mode), each with what to change to see them.
 */
export function resultsSummary(
  f: Pick<BrowseFilters, "kind" | "content" | "compatibleOnly"> & Partial<Pick<BrowseFilters, "runsOnMyCard">>,
  shown: number,
  totals: BrowseTotals,
): { count: string; hints: string[] } {
  const models = f.kind === "models";
  const noun = models ? (shown === 1 ? "model" : "models") : shown === 1 ? "style add-on" : "style add-ons";
  const hints: string[] = [];
  if (f.compatibleOnly)
    hints.push(models ? "Showing models that run in Pinhole — turn off “Works with Pinhole” to see all." : "Showing style add-ons that work in Pinhole — turn off “Works with Pinhole” to see all.");
  if (f.content === "safe" && totals.hiddenByContent > 0)
    hints.push(`Safe mode hid ${totals.hiddenByContent} made for adults.`);
  if (models && f.runsOnMyCard && totals.hiddenBySize > 0)
    hints.push(`“Runs on my card” hid ${totals.hiddenBySize} too big for your graphics card.`);
  return { count: `${shown.toLocaleString("en-US")} ${noun}`, hints };
}
